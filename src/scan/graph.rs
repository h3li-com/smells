use std::collections::btree_map::Entry;

use super::{
    Attribute, BTreeMap, BTreeSet, Component, Counter, Expr, ExprLit, Facts, Input, Item, ItemMod,
    Lit, Meta, Module, Path, PathBuf, Report, Result, Type, UseGroup, UseName, UseRename, UseTree,
    location, metrics, syntax,
};

fn clean_path(path: &Path) -> Result<String, String> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(value) => result.push(value),
            Component::CurDir => {}
            Component::ParentDir if result.pop() => {}
            _ => return Err("module path escapes source corpus".into()),
        }
    }
    Ok(result
        .to_str()
        .ok_or("module path is not UTF-8")?
        .replace('\\', "/"))
}

fn path_attribute(item: &ItemMod) -> Result<Option<String>, String> {
    let mut found = None;
    for attr in &item.attrs {
        if conditional_path_attribute(attr) {
            return Err("conditional module path requires active-cfg analysis".into());
        }
        if attr.path().is_ident("path") {
            found = authored_path_attribute(attr, found.as_deref())?;
        }
    }
    Ok(found)
}

fn conditional_path_attribute(attribute: &Attribute) -> bool {
    attribute.path().is_ident("cfg_attr")
        && matches!(&attribute.meta, Meta::List(list) if mentions_path(list.tokens.clone()))
}

fn authored_path_attribute(
    attribute: &Attribute,
    found: Option<&str>,
) -> Result<Option<String>, String> {
    let Meta::NameValue(value) = &attribute.meta else {
        return Err("unsupported module path attribute".into());
    };
    let Expr::Lit(ExprLit {
        lit: Lit::Str(path),
        ..
    }) = &value.value
    else {
        return Err("unsupported module path attribute".into());
    };
    if found.is_some() || path.value().contains('\\') || Path::new(&path.value()).is_absolute() {
        return Err("ambiguous or non-portable module path attribute".into());
    }
    Ok(Some(path.value()))
}

fn mentions_path(tokens: proc_macro2::TokenStream) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(ident) => ident == "path",
        proc_macro2::TokenTree::Group(group) => mentions_path(group.stream()),
        _ => false,
    })
}

fn inline_base(item: &ItemMod, base: &Path, attribute_base: &Path) -> Result<PathBuf, String> {
    let next = match path_attribute(item)? {
        Some(path) => attribute_base.join(path),
        None => base.join(item.ident.to_string()),
    };
    Ok(PathBuf::from(clean_path(&next)?))
}

fn module_file(
    item: &ItemMod,
    base: &Path,
    attribute_base: &Path,
    parsed: &BTreeMap<String, syn::File>,
) -> Result<String, String> {
    if let Some(path) = path_attribute(item)? {
        let path = clean_path(&attribute_base.join(path))?;
        if parsed.contains_key(&path) {
            return Ok(path);
        }
        return Err(format!("module source missing from corpus: {path}"));
    }
    let name = item.ident.to_string();
    let candidates = [
        clean_path(&base.join(format!("{name}.rs")))?,
        clean_path(&base.join(&name).join("mod.rs"))?,
    ];
    let mut found = candidates.into_iter().filter(|p| parsed.contains_key(p));
    match (found.next(), found.next()) {
        (Some(path), None) => Ok(path),
        _ => Err(format!("missing or ambiguous module source: {name}")),
    }
}

/// Whether `path` is a Cargo target root that is not named `lib.rs` or
/// `main.rs`: a file directly in a package's `tests`, `examples` or `benches`
/// directory, in `src/bin`, or a package's `build.rs`. Each is its own crate,
/// so its `mod` declarations resolve next to it.
fn cargo_target_root(path: &str) -> bool {
    let path = Path::new(path);
    let Some(parent) = path.parent() else {
        return false;
    };
    let ancestors_outside_src = || {
        parent
            .components()
            .all(|component| component.as_os_str() != "src")
    };
    match (
        path.file_name().and_then(|name| name.to_str()),
        parent.file_name().and_then(|name| name.to_str()),
    ) {
        (Some("build.rs"), _) | (_, Some("tests" | "examples" | "benches")) => {
            ancestors_outside_src()
        }
        (_, Some("bin")) => parent
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "src"),
        _ => false,
    }
}

/// The directory that holds the modules a crate root declares.
fn root_base(path: &str) -> PathBuf {
    if cargo_target_root(path) {
        Path::new(path).parent().unwrap_or(Path::new("")).into()
    } else {
        file_base(path)
    }
}

fn file_base(path: &str) -> PathBuf {
    let path = Path::new(path);
    let parent = path.parent().unwrap_or(Path::new(""));
    if matches!(
        path.file_name().and_then(|p| p.to_str()),
        Some("lib.rs" | "main.rs" | "mod.rs")
    ) {
        parent.into()
    } else {
        parent.join(path.file_stem().unwrap_or_default())
    }
}

fn references(
    items: &[Item],
    base: &Path,
    attribute_base: &Path,
    parsed: &BTreeMap<String, syn::File>,
    found: &mut BTreeSet<String>,
    depth: usize,
) -> Result<(), String> {
    if depth > 128 {
        return Err("module nesting limit exceeded".into());
    }
    for item in items {
        if let Item::Mod(module) = item {
            if let Some((_, items)) = &module.content {
                let next = inline_base(module, base, attribute_base)?;
                references(items, &next, &next, parsed, found, depth + 1)?;
            } else {
                found.insert(module_file(module, base, attribute_base, parsed)?);
            }
        }
    }
    Ok(())
}

fn imports(
    tree: &UseTree,
    prefix: &[String],
    result: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix.to_vec();
            next.push(path.ident.to_string());
            imports(&path.tree, &next, result)
        }
        UseTree::Group(group) => import_group(group, prefix, result),
        UseTree::Name(name) => import_name(name, prefix, result),
        UseTree::Rename(rename) => {
            import_rename(rename, prefix, result);
            Ok(())
        }
        UseTree::Glob(_) => {
            result.insert(format!("*{}", result.len()), prefix.to_vec());
            Ok(())
        }
    }
}

fn import_group(
    group: &UseGroup,
    prefix: &[String],
    result: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    group
        .items
        .iter()
        .try_for_each(|tree| imports(tree, prefix, result))
}

/// The binding recorded for a name that two imports bring in, such as
/// `pub use self::from_ref::FromRef;` and `pub use axum_macros::FromRef;`.
/// Rust allows that when the items live in different namespaces (a trait and a
/// derive macro), which this scan cannot tell apart, so only resolving
/// through the name is an error.
const AMBIGUOUS_IMPORT: &str = "\0ambiguous";
const EXTERNAL_CRATE: &str = "\0extern";

fn insert_import(alias: String, mut path: Vec<String>, result: &mut BTreeMap<String, Vec<String>>) {
    // `pub use http;` re-exports an external crate under its own name. Rooting it
    // outside the crate keeps `http::…` and `crate::http::…` external instead of
    // resolving the binding to itself.
    if path.len() == 1 && path[0] == alias {
        path.insert(0, EXTERNAL_CRATE.into());
    }
    match result.entry(alias) {
        Entry::Occupied(mut entry) => {
            if *entry.get() != path {
                entry.insert(vec![AMBIGUOUS_IMPORT.into()]);
            }
        }
        Entry::Vacant(entry) => {
            entry.insert(path);
        }
    }
}

/// The path an import binds `name` to, or an error when two imports bind it.
fn imported_path<'a>(
    imports: &'a BTreeMap<String, Vec<String>>,
    name: &str,
) -> Result<Option<&'a Vec<String>>, String> {
    match imports.get(name) {
        Some(path) if path.first().is_some_and(|first| first == AMBIGUOUS_IMPORT) => {
            Err(format!("ambiguous import binding: {name}"))
        }
        path => Ok(path),
    }
}

fn import_name(
    name: &UseName,
    prefix: &[String],
    result: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    let mut path = prefix.to_vec();
    let name = name.ident.to_string();
    let alias = if name == "self" {
        path.last().cloned().ok_or("invalid self import")?
    } else {
        path.push(name.clone());
        name
    };
    insert_import(alias, path, result);
    Ok(())
}

fn import_rename(
    rename: &UseRename,
    prefix: &[String],
    result: &mut BTreeMap<String, Vec<String>>,
) {
    let mut path = prefix.to_vec();
    if rename.ident != "self" {
        path.push(rename.ident.to_string());
    }
    let alias = rename.rename.to_string();
    if alias != "_" {
        insert_import(alias, path, result);
    }
}

struct ModuleWalker<'a> {
    root: &'a str,
    parsed: &'a BTreeMap<String, syn::File>,
    stack: Vec<String>,
    result: &'a mut BTreeMap<String, Module>,
    /// Inside a test target root or a `#[cfg(test)]` module.
    test: bool,
}

impl ModuleWalker<'_> {
    fn bindings(items: &[Item]) -> Result<BTreeMap<String, Vec<String>>, String> {
        let mut bindings = BTreeMap::new();
        for item in items {
            if let Item::Use(item) = item {
                imports(&item.tree, &[], &mut bindings)?;
            }
        }
        Ok(bindings)
    }

    fn insert(&mut self, path: &str, key: &str, items: &[Item]) -> Result<(), String> {
        let module = Module {
            key: key.into(),
            root: self.root.into(),
            path: path.into(),
            items: items.to_vec(),
            imports: Self::bindings(items)?,
            test: self.test,
        };
        if self.result.insert(key.into(), module).is_some() {
            Err(format!(
                "ambiguous authored module: {key}; active-cfg analysis is not implemented"
            ))
        } else {
            Ok(())
        }
    }

    fn visit_child(
        &mut self,
        module: &ItemMod,
        path: &str,
        key: &str,
        base: &Path,
        attribute_base: &Path,
    ) -> Result<(), String> {
        let enclosing_test = self.test;
        self.test |= crate::core::test_suite::rust_cfg_test(&module.attrs);
        let result = self.visit_child_items(module, path, key, base, attribute_base);
        self.test = enclosing_test;
        result
    }

    fn visit_child_items(
        &mut self,
        module: &ItemMod,
        path: &str,
        key: &str,
        base: &Path,
        attribute_base: &Path,
    ) -> Result<(), String> {
        let next_key = format!("{key}::{}", module.ident);
        if let Some((_, items)) = &module.content {
            let next = inline_base(module, base, attribute_base)?;
            return self.visit(path, &next_key, items, &next, &next);
        }
        let next_path = module_file(module, base, attribute_base, self.parsed)?;
        if self.stack.contains(&next_path) {
            return Err(format!("cyclic module source: {next_path}"));
        }
        self.stack.push(next_path.clone());
        let result = self.visit(
            &next_path,
            &next_key,
            &self.parsed[&next_path].items,
            &file_base(&next_path),
            Path::new(&next_path).parent().unwrap_or(Path::new("")),
        );
        self.stack.pop();
        result
    }

    fn visit(
        &mut self,
        path: &str,
        key: &str,
        items: &[Item],
        base: &Path,
        attribute_base: &Path,
    ) -> Result<(), String> {
        if self.stack.len() > 128
            || key
                .strip_prefix(self.root)
                .unwrap_or(key)
                .matches("::")
                .count()
                > 128
        {
            return Err("module nesting limit exceeded".into());
        }
        self.insert(path, key, items)?;
        for item in items {
            if let Item::Mod(module) = item {
                self.visit_child(module, path, key, base, attribute_base)?;
            }
        }
        Ok(())
    }
}

enum OwnerLookup {
    Missing,
    Resolved(Option<String>),
}

enum ResolutionStart {
    Continue {
        prefix: String,
        remaining: Vec<String>,
    },
    Resolved(Option<String>),
}

impl Facts {
    pub(crate) fn owner(&self, ty: &Type, module: &str) -> Result<Option<String>, String> {
        match ty {
            Type::Path(path) if path.qself.is_none() => self.resolve(
                &path
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>(),
                module,
                0,
            ),
            Type::Array(_)
            | Type::Slice(_)
            | Type::Tuple(_)
            | Type::Reference(_)
            | Type::Ptr(_) => Ok(None),
            _ => Err(format!(
                "unsupported impl target in {module}: {}",
                syntax(ty)
            )),
        }
    }
    pub(crate) fn resolve(
        &self,
        path: &[String],
        module: &str,
        depth: usize,
    ) -> Result<Option<String>, String> {
        self.resolve_bounded(path, module, depth, &mut 0)
    }

    fn super_prefix(
        path: &[String],
        module: &str,
        source_root: &str,
    ) -> Result<(String, usize), String> {
        let consumed = path
            .iter()
            .position(|part| part != "super")
            .unwrap_or(path.len());
        let prefix = (0..consumed).try_fold(module.to_string(), |prefix, _| {
            parent_module(&prefix, source_root, module)
        })?;
        Ok((prefix, consumed))
    }

    fn resolution_start(
        &self,
        path: &[String],
        module: &str,
        depth: usize,
        visits: &mut usize,
    ) -> Result<ResolutionStart, String> {
        let context = &self.modules[module];
        let first = path.first().ok_or("empty type path")?;
        let (prefix, consumed) = match first.as_str() {
            "crate" => (context.root.clone(), 1),
            "self" => (module.to_string(), 1),
            "super" => Self::super_prefix(path, module, &context.root)?,
            _ => {
                if let Some(import) = imported_path(&context.imports, first)? {
                    let mut next = import.clone();
                    next.extend_from_slice(&path[1..]);
                    return self
                        .resolve_bounded(&next, module, depth + 1, visits)
                        .map(ResolutionStart::Resolved);
                }
                (module.to_string(), 0)
            }
        };
        Ok(ResolutionStart::Continue {
            prefix,
            remaining: path[consumed..].to_vec(),
        })
    }

    fn exact_owner(
        &self,
        key: &str,
        depth: usize,
        visits: &mut usize,
    ) -> Result<OwnerLookup, String> {
        if self.types.contains_key(key) {
            return Ok(OwnerLookup::Resolved(Some(key.to_string())));
        }
        let Some((module, alias)) = self.aliases.get(key) else {
            return Ok(OwnerLookup::Missing);
        };
        match alias {
            Type::Path(path) if path.qself.is_none() => self
                .resolve_bounded(
                    &path
                        .path
                        .segments
                        .iter()
                        .map(|segment| segment.ident.to_string())
                        .collect::<Vec<_>>(),
                    module,
                    depth + 1,
                    visits,
                )
                .map(OwnerLookup::Resolved),
            _ => Ok(OwnerLookup::Resolved(None)),
        }
    }

    fn imported_owner(
        &self,
        prefix: &str,
        remaining: &[String],
        depth: usize,
        visits: &mut usize,
    ) -> Result<(OwnerLookup, String), String> {
        let mut parent = prefix.to_string();
        for (index, part) in remaining.iter().enumerate() {
            let imports = self.modules.get(&parent).map(|module| &module.imports);
            if let Some(import) = imports
                .map(|imports| imported_path(imports, part))
                .transpose()?
                .flatten()
            {
                let mut next = import.clone();
                next.extend_from_slice(&remaining[index + 1..]);
                let owner = self.resolve_bounded(&next, &parent, depth + 1, visits)?;
                return Ok((OwnerLookup::Resolved(owner), parent));
            }
            if index + 1 < remaining.len() {
                parent = format!("{parent}::{part}");
            }
        }
        Ok((OwnerLookup::Missing, parent))
    }

    fn glob_owner(
        &self,
        parent: &str,
        remaining: &[String],
        module: &str,
        original: &[String],
        depth: usize,
        visits: &mut usize,
    ) -> Result<OwnerLookup, String> {
        let leaf = remaining.last().ok_or("empty resolved type path")?;
        let candidates = self.glob_matches(parent, leaf, depth, visits)?;
        let external = candidates.iter().any(Option::is_none);
        let mut matches: Vec<String> = candidates.into_iter().flatten().collect();
        matches.sort();
        matches.dedup();
        if matches.len() > 1 {
            return Err(format!(
                "ambiguous glob owner in {module}: {}",
                original.join("::")
            ));
        }
        // A glob that only reaches a foreign path binds a type this scan cannot see.
        Ok(match matches.pop() {
            Some(owner) => OwnerLookup::Resolved(Some(owner)),
            None if external => OwnerLookup::Resolved(None),
            None => OwnerLookup::Missing,
        })
    }

    fn glob_matches(
        &self,
        parent: &str,
        leaf: &str,
        depth: usize,
        visits: &mut usize,
    ) -> Result<Vec<Option<String>>, String> {
        let mut matches = vec![];
        let Some(context) = self.modules.get(parent) else {
            return Ok(matches);
        };
        for (name, import) in &context.imports {
            if name.starts_with('*') {
                self.glob_match(import, leaf, (parent, depth), visits, &mut matches)?;
            }
        }
        Ok(matches)
    }

    /// Pushes the owner `leaf` resolves to through one glob import, if it resolves.
    fn glob_match(
        &self,
        import: &[String],
        leaf: &str,
        (parent, depth): (&str, usize),
        visits: &mut usize,
        matches: &mut Vec<Option<String>>,
    ) -> Result<(), String> {
        let mut candidate = import.to_vec();
        candidate.push(leaf.to_string());
        match self.resolve_bounded(&candidate, parent, depth + 1, visits) {
            Ok(owner) => matches.push(owner),
            Err(error) if error.contains("cycle/budget") || error.starts_with("ambiguous") => {
                return Err(error);
            }
            Err(_) => {}
        }
        Ok(())
    }

    fn unresolved_owner(
        &self,
        first: &str,
        path: &[String],
        module: &str,
    ) -> Result<Option<String>, String> {
        let primitive = matches!(
            first,
            "bool"
                | "char"
                | "str"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
        );
        // The standard prelude names these without an import.
        let prelude =
            path.len() == 1 && matches!(first, "Box" | "Option" | "Result" | "String" | "Vec");
        let external = matches!(first, "std" | "core" | "alloc")
            || (path.len() > 1
                && !self.modules.contains_key(&format!("{module}::{first}"))
                && !matches!(first, "crate" | "self" | "super"));
        if primitive || prelude || external {
            Ok(None)
        } else {
            Err(format!(
                "unresolved local impl owner in {module}: {}",
                path.join("::")
            ))
        }
    }

    fn resolved_owner(
        &self,
        prefix: &str,
        remaining: &[String],
        module: &str,
        original: &[String],
        depth: usize,
        visits: &mut usize,
    ) -> Result<OwnerLookup, String> {
        let key = format!("{prefix}::{}", remaining.join("::"));
        if let OwnerLookup::Resolved(owner) = self.exact_owner(&key, depth, visits)? {
            return Ok(OwnerLookup::Resolved(owner));
        }
        let (imported, parent) = self.imported_owner(prefix, remaining, depth, visits)?;
        if let OwnerLookup::Resolved(owner) = imported {
            return Ok(OwnerLookup::Resolved(owner));
        }
        self.glob_owner(&parent, remaining, module, original, depth, visits)
    }

    fn resolve_bounded(
        &self,
        path: &[String],
        module: &str,
        depth: usize,
        visits: &mut usize,
    ) -> Result<Option<String>, String> {
        consume_resolution_budget(depth, visits, module)?;
        let first = path.first().ok_or("empty type path")?;
        let (prefix, remaining) = match self.resolution_start(path, module, depth, visits)? {
            ResolutionStart::Continue { prefix, remaining } => (prefix, remaining),
            ResolutionStart::Resolved(owner) => return Ok(owner),
        };
        if let OwnerLookup::Resolved(owner) =
            self.resolved_owner(&prefix, &remaining, module, path, depth, visits)?
        {
            return Ok(owner);
        }
        self.unresolved_owner(first, path, module)
    }
}

fn parent_module(prefix: &str, source_root: &str, module: &str) -> Result<String, String> {
    if prefix == source_root {
        return Err(format!("super escapes source root: {module}"));
    }
    prefix
        .rsplit_once("::")
        .map(|(parent, _)| parent.to_string())
        .ok_or_else(|| "invalid module parent".into())
}

fn consume_resolution_budget(depth: usize, visits: &mut usize, module: &str) -> Result<(), String> {
    *visits += 1;
    if depth > 64 || *visits > 4096 {
        Err(format!("owner resolution cycle/budget in {module}"))
    } else {
        Ok(())
    }
}

pub(super) fn parse_sources(input: &Input, report: &mut Report) -> BTreeMap<String, syn::File> {
    let mut parsed = BTreeMap::new();
    for (path, source) in &input.files {
        match syn::parse_file(source) {
            Ok(file) => {
                metrics::add(
                    Counter::SyntaxTokens,
                    rustc_lexer::tokenize(source, rustc_lexer::FrontmatterAllowed::No).count(),
                );
                parsed.insert(path.clone(), file);
            }
            Err(error) => {
                report.error_at(
                    super::parse_error_message(path, &error),
                    location(path, error.span()),
                );
            }
        }
    }
    parsed
}

fn file_references(
    path: &str,
    file: &syn::File,
    base: &Path,
    parsed: &BTreeMap<String, syn::File>,
) -> Result<BTreeSet<String>, String> {
    let mut found = BTreeSet::new();
    let attribute_base = Path::new(path).parent().unwrap_or(Path::new(""));
    references(&file.items, base, attribute_base, parsed, &mut found, 0)?;
    Ok(found)
}

pub(super) fn referenced_sources(
    parsed: &BTreeMap<String, syn::File>,
    report: &mut Report,
) -> BTreeSet<String> {
    // A Cargo target root such as `tests/cli.rs` resolves its modules next to
    // itself, unless another file declares it as a module after all.
    let mut found: BTreeMap<&str, Result<BTreeSet<String>, String>> = parsed
        .iter()
        .map(|(path, file)| {
            let references = file_references(path, file, &root_base(path), parsed);
            (path.as_str(), references)
        })
        .collect();
    let referenced: BTreeSet<String> = found
        .values()
        .filter_map(|references| references.as_ref().ok())
        .flatten()
        .cloned()
        .collect();
    for (path, file) in parsed {
        if cargo_target_root(path) && referenced.contains(path) {
            found.insert(path, file_references(path, file, &file_base(path), parsed));
        }
    }
    let mut referenced = BTreeSet::new();
    for (path, references) in found {
        match references {
            Ok(paths) => referenced.extend(paths),
            Err(error) => report.error(format!("{path}: {error}")),
        }
    }
    referenced
}

pub(super) fn root_sources(
    parsed: &BTreeMap<String, syn::File>,
    referenced: &BTreeSet<String>,
) -> Vec<String> {
    parsed
        .keys()
        .filter(|path| !referenced.contains(*path))
        .cloned()
        .collect()
}

pub(super) fn collect_modules(
    roots: Vec<String>,
    parsed: &BTreeMap<String, syn::File>,
    report: &mut Report,
) -> BTreeMap<String, Module> {
    let mut modules = BTreeMap::new();
    for root in roots {
        let mut walker = ModuleWalker {
            root: &root,
            parsed,
            stack: vec![root.clone()],
            result: &mut modules,
            test: crate::core::test_suite::conventional_test_path(&root),
        };
        if let Err(error) = walker.visit(
            &root,
            &root,
            &parsed[&root].items,
            &root_base(&root),
            Path::new(&root).parent().unwrap_or(Path::new("")),
        ) {
            report.error(error);
        }
    }
    modules
}

pub(super) fn validate_module_coverage(
    modules: &BTreeMap<String, Module>,
    parsed: &BTreeMap<String, syn::File>,
) -> Result<(), String> {
    let included: BTreeSet<_> = modules.values().map(|module| &module.path).collect();
    if included.len() == parsed.len() {
        Ok(())
    } else {
        Err("unreachable/cyclic module source omitted from traversal".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Facts, Module, ModuleWalker, cargo_target_root, clean_path, conditional_path_attribute,
        consume_resolution_budget, imported_path, imports, path_attribute, references, root_base,
        validate_module_coverage,
    };
    use crate::{report::Location, scan::TypeFacts};
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::Path,
    };

    #[test]
    fn clean_path_normalizes_local_segments_without_allowing_repository_escape() {
        assert_eq!(clean_path(Path::new("./src/lib.rs")).unwrap(), "src/lib.rs");
        assert_eq!(
            clean_path(Path::new("src/./domain/../lib.rs")).unwrap(),
            "src/lib.rs"
        );
        assert_eq!(clean_path(Path::new("src/../lib.rs")).unwrap(), "lib.rs");
        assert_eq!(
            clean_path(Path::new("../outside.rs")).unwrap_err(),
            "module path escapes source corpus"
        );
        assert_eq!(
            clean_path(Path::new("src/../../outside.rs")).unwrap_err(),
            "module path escapes source corpus"
        );
        assert_eq!(
            clean_path(Path::new("/absolute.rs")).unwrap_err(),
            "module path escapes source corpus"
        );
    }

    #[test]
    fn module_path_attributes_are_literal_unique_portable_and_unconditional() {
        let explicit: syn::ItemMod = syn::parse_quote!(
            #[path = "custom/domain.rs"]
            mod domain;
        );
        assert_eq!(
            path_attribute(&explicit).unwrap().as_deref(),
            Some("custom/domain.rs")
        );

        let duplicate: syn::ItemMod = syn::parse_quote!(
            #[path = "first.rs"]
            #[path = "second.rs"]
            mod domain;
        );
        assert!(
            path_attribute(&duplicate)
                .unwrap_err()
                .contains("ambiguous")
        );

        for item in [
            syn::parse_quote!(
                #[path = "..\\outside.rs"]
                mod domain;
            ),
            syn::parse_quote!(
                #[path = "/outside.rs"]
                mod domain;
            ),
            syn::parse_quote!(
                #[path("dynamic.rs")]
                mod domain;
            ),
        ] {
            assert!(path_attribute(&item).is_err());
        }

        let conditional: syn::ItemMod = syn::parse_quote!(
            #[cfg_attr(unix, path = "unix.rs")]
            mod domain;
        );
        assert!(conditional_path_attribute(&conditional.attrs[0]));
        assert!(
            path_attribute(&conditional)
                .unwrap_err()
                .contains("active-cfg")
        );

        let unrelated: syn::ItemMod = syn::parse_quote!(
            #[cfg_attr(unix, allow(dead_code))]
            mod domain;
        );
        assert!(!conditional_path_attribute(&unrelated.attrs[0]));
        assert_eq!(path_attribute(&unrelated).unwrap(), None);

        let nested_path_token: syn::ItemMod = syn::parse_quote!(
            #[cfg_attr(any(unix, path), allow(dead_code))]
            mod domain;
        );
        assert!(conditional_path_attribute(&nested_path_token.attrs[0]));
    }

    #[test]
    fn inline_module_nesting_limit_is_strict_and_charged_per_level() {
        let parsed = BTreeMap::new();
        let mut found = BTreeSet::new();
        assert!(
            references(
                &[],
                Path::new("src"),
                Path::new("src"),
                &parsed,
                &mut found,
                128,
            )
            .is_ok()
        );
        assert_eq!(
            references(
                &[],
                Path::new("src"),
                Path::new("src"),
                &parsed,
                &mut found,
                129,
            )
            .unwrap_err(),
            "module nesting limit exceeded"
        );

        let source = format!("{}{}", "mod nested {".repeat(3), "}".repeat(3));
        let file = syn::parse_file(&source).unwrap();
        assert_eq!(
            references(
                &file.items,
                Path::new("src"),
                Path::new("src"),
                &parsed,
                &mut found,
                127,
            )
            .unwrap_err(),
            "module nesting limit exceeded"
        );
    }

    #[test]
    fn module_walker_limits_file_cycles_and_inline_keys_independently() {
        let parsed = BTreeMap::new();
        let mut result = BTreeMap::new();
        let mut walker = ModuleWalker {
            root: "crate",
            parsed: &parsed,
            stack: vec!["path".into(); 129],
            result: &mut result,
            test: false,
        };
        assert_eq!(
            walker
                .visit(
                    "src/lib.rs",
                    "crate",
                    &[],
                    Path::new("src"),
                    Path::new("src"),
                )
                .unwrap_err(),
            "module nesting limit exceeded"
        );

        let mut result = BTreeMap::new();
        let mut walker = ModuleWalker {
            root: "crate",
            parsed: &parsed,
            stack: Vec::new(),
            result: &mut result,
            test: false,
        };
        let deep_key = format!("crate{}", "::nested".repeat(129));
        assert_eq!(
            walker
                .visit(
                    "src/lib.rs",
                    &deep_key,
                    &[],
                    Path::new("src"),
                    Path::new("src"),
                )
                .unwrap_err(),
            "module nesting limit exceeded"
        );

        let mut result = BTreeMap::new();
        let mut walker = ModuleWalker {
            root: "crate",
            parsed: &parsed,
            stack: vec!["path".into(); 128],
            result: &mut result,
            test: false,
        };
        let boundary_key = format!("crate{}", "::nested".repeat(128));
        assert!(
            walker
                .visit(
                    "src/lib.rs",
                    &boundary_key,
                    &[],
                    Path::new("src"),
                    Path::new("src"),
                )
                .is_ok()
        );
    }

    #[test]
    fn impl_owner_resolution_rejects_qualified_and_unsupported_targets_but_ignores_wrappers() {
        let facts = Facts {
            modules: BTreeMap::new(),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        for wrapper in [
            syn::parse_quote!([Account; 2]),
            syn::parse_quote!([Account]),
            syn::parse_quote!((Account, Invoice)),
            syn::parse_quote!(&Account),
            syn::parse_quote!(*const Account),
        ] {
            assert_eq!(facts.owner(&wrapper, "crate").unwrap(), None);
        }

        let qualified: syn::Type = syn::parse_quote!(<Account as Port>::Item);
        assert!(
            facts
                .owner(&qualified, "crate")
                .unwrap_err()
                .contains("unsupported impl target")
        );
        let inferred: syn::Type = syn::parse_quote!(impl Port);
        assert!(
            facts
                .owner(&inferred, "crate")
                .unwrap_err()
                .contains("unsupported impl target")
        );
    }

    #[test]
    fn type_resolution_distinguishes_crate_self_super_and_import_prefixes() {
        let location = Location {
            path: "src/lib.rs".into(),
            line: 1,
            column: 1,
        };
        let type_fact = || TypeFacts {
            location: location.clone(),
            fields: Vec::new(),
            field_count: 0,
            is_enum: false,
            variants: BTreeSet::new(),
            functions: Vec::new(),
        };
        let facts = Facts {
            modules: BTreeMap::from([(
                "crate::domain".into(),
                Module {
                    key: "crate::domain".into(),
                    root: "crate".into(),
                    path: "src/domain.rs".into(),
                    items: Vec::new(),
                    test: false,
                    imports: BTreeMap::from([(
                        "AccountAlias".into(),
                        vec!["crate".into(), "model".into(), "Account".into()],
                    )]),
                },
            )]),
            types: BTreeMap::from([
                ("crate::domain::Local".into(), type_fact()),
                ("crate::model::Account".into(), type_fact()),
                ("crate::Parent".into(), type_fact()),
            ]),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };

        for (path, expected) in [
            (&["self", "Local"][..], "crate::domain::Local"),
            (&["crate", "model", "Account"][..], "crate::model::Account"),
            (&["super", "Parent"][..], "crate::Parent"),
            (&["AccountAlias"][..], "crate::model::Account"),
        ] {
            let path = path.iter().map(|part| (*part).into()).collect::<Vec<_>>();
            assert_eq!(
                facts.resolve(&path, "crate::domain", 0).unwrap().as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn alias_resolution_rejects_qualified_targets() {
        let location = Location {
            path: "src/lib.rs".into(),
            line: 1,
            column: 1,
        };
        let facts = Facts {
            modules: BTreeMap::from([(
                "crate".into(),
                Module {
                    key: "crate".into(),
                    root: "crate".into(),
                    path: "src/lib.rs".into(),
                    items: Vec::new(),
                    imports: BTreeMap::new(),
                    test: false,
                },
            )]),
            types: BTreeMap::from([(
                "crate::Port::Item".into(),
                TypeFacts {
                    location,
                    fields: Vec::new(),
                    field_count: 0,
                    is_enum: false,
                    variants: BTreeSet::new(),
                    functions: Vec::new(),
                },
            )]),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::from([(
                "crate::Alias".into(),
                ("crate".into(), syn::parse_quote!(<Account as Port>::Item)),
            )]),
        };

        assert_eq!(facts.resolve(&["Alias".into()], "crate", 0).unwrap(), None);
    }

    #[test]
    fn cyclic_imports_charge_each_recursive_resolution_step() {
        let facts = Facts {
            modules: BTreeMap::from([(
                "crate".into(),
                Module {
                    key: "crate".into(),
                    root: "crate".into(),
                    path: "src/lib.rs".into(),
                    items: Vec::new(),
                    imports: BTreeMap::from([("Alias".into(), vec!["Alias".into()])]),
                    test: false,
                },
            )]),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        let mut visits = 0;

        let error = facts
            .resolve_bounded(&["Alias".into()], "crate", 63, &mut visits)
            .unwrap_err();

        assert!(error.contains("cycle/budget"));
        assert_eq!(visits, 3);
    }

    #[test]
    fn cyclic_aliases_and_nested_imports_charge_their_own_resolution_step() {
        let module = Module {
            key: "crate".into(),
            root: "crate".into(),
            path: "src/lib.rs".into(),
            items: Vec::new(),
            imports: BTreeMap::new(),
            test: false,
        };
        let aliases = Facts {
            modules: BTreeMap::from([("crate".into(), module.clone())]),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::from([(
                "crate::Alias".into(),
                ("crate".into(), syn::parse_quote!(Alias)),
            )]),
        };
        let mut alias_visits = 0;
        assert!(
            aliases
                .exact_owner("crate::Alias", 63, &mut alias_visits)
                .err()
                .unwrap()
                .contains("cycle/budget")
        );
        assert_eq!(alias_visits, 2);

        let imports = Facts {
            modules: BTreeMap::from([(
                "crate".into(),
                Module {
                    imports: BTreeMap::from([("Alias".into(), vec!["Alias".into()])]),
                    ..module
                },
            )]),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        let mut import_visits = 0;
        assert!(
            imports
                .imported_owner("crate", &["Alias".into()], 63, &mut import_visits)
                .err()
                .unwrap()
                .contains("cycle/budget")
        );
        assert_eq!(import_visits, 2);
    }

    #[test]
    fn glob_resolution_propagates_only_budget_and_ambiguity_errors() {
        let module = |imports| Module {
            key: "crate".into(),
            root: "crate".into(),
            path: "src/lib.rs".into(),
            items: Vec::new(),
            imports,
            test: false,
        };
        let empty = Facts {
            modules: BTreeMap::from([("crate".into(), module(BTreeMap::new()))]),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        let mut missing = Vec::new();
        empty
            .glob_match(&[], "Missing", ("crate", 0), &mut 0, &mut missing)
            .unwrap();
        assert!(missing.is_empty());

        let cyclic = Facts {
            modules: BTreeMap::from([(
                "crate".into(),
                module(BTreeMap::from([("Alias".into(), vec!["Alias".into()])])),
            )]),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        let mut cyclic_visits = 0;
        assert!(
            cyclic
                .glob_match(
                    &[],
                    "Alias",
                    ("crate", 63),
                    &mut cyclic_visits,
                    &mut Vec::new()
                )
                .unwrap_err()
                .contains("cycle/budget")
        );
        assert_eq!(cyclic_visits, 2);

        let location = Location {
            path: "src/lib.rs".into(),
            line: 1,
            column: 1,
        };
        let type_fact = || TypeFacts {
            location: location.clone(),
            fields: Vec::new(),
            field_count: 0,
            is_enum: false,
            variants: BTreeSet::new(),
            functions: Vec::new(),
        };
        let ambiguous = Facts {
            modules: BTreeMap::from([(
                "crate".into(),
                module(BTreeMap::from([
                    ("*0".into(), vec!["crate".into(), "first".into()]),
                    ("*1".into(), vec!["crate".into(), "second".into()]),
                ])),
            )]),
            types: BTreeMap::from([
                ("crate::first::Thing".into(), type_fact()),
                ("crate::second::Thing".into(), type_fact()),
            ]),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        assert!(
            ambiguous
                .glob_match(&[], "Thing", ("crate", 0), &mut 0, &mut Vec::new())
                .unwrap_err()
                .starts_with("ambiguous")
        );
    }

    #[test]
    fn unresolved_owner_distinguishes_primitives_external_paths_and_local_paths() {
        let module = |key: &str| Module {
            key: key.into(),
            root: "crate".into(),
            path: "src/lib.rs".into(),
            items: Vec::new(),
            imports: BTreeMap::new(),
            test: false,
        };
        let facts = Facts {
            modules: BTreeMap::from([
                ("crate".into(), module("crate")),
                ("crate::local".into(), module("crate::local")),
            ]),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };

        assert_eq!(
            facts
                .unresolved_owner("i32", &["i32".into()], "crate")
                .unwrap(),
            None
        );
        for (first, path) in [
            ("std", vec!["std".into()]),
            ("core", vec!["core".into()]),
            ("alloc", vec!["alloc".into()]),
            ("dependency", vec!["dependency".into(), "Thing".into()]),
        ] {
            assert_eq!(facts.unresolved_owner(first, &path, "crate").unwrap(), None);
        }
        for (first, path) in [
            ("Missing", vec!["Missing".into()]),
            ("local", vec!["local".into(), "Thing".into()]),
            ("crate", vec!["crate".into(), "Thing".into()]),
            ("self", vec!["self".into(), "Thing".into()]),
            ("super", vec!["super".into(), "Thing".into()]),
        ] {
            assert!(
                facts
                    .unresolved_owner(first, &path, "crate")
                    .unwrap_err()
                    .contains("unresolved local impl owner")
            );
        }
    }

    #[test]
    fn resolution_budget_enforces_depth_and_visit_boundaries_independently() {
        let mut visits = 4095;
        consume_resolution_budget(64, &mut visits, "crate").unwrap();
        assert_eq!(visits, 4096);

        let mut depth_visits = 0;
        assert!(
            consume_resolution_budget(65, &mut depth_visits, "crate")
                .unwrap_err()
                .contains("cycle/budget")
        );
        assert_eq!(depth_visits, 1);

        let mut visit_limit = 4096;
        assert!(
            consume_resolution_budget(0, &mut visit_limit, "crate")
                .unwrap_err()
                .contains("cycle/budget")
        );
        assert_eq!(visit_limit, 4097);
    }

    #[test]
    fn module_coverage_requires_every_parsed_source_to_be_reachable() {
        let parsed = BTreeMap::from([
            ("src/lib.rs".into(), syn::parse_file("").unwrap()),
            ("src/orphan.rs".into(), syn::parse_file("").unwrap()),
        ]);
        let module = |key: &str, path: &str| Module {
            key: key.into(),
            root: "crate".into(),
            path: path.into(),
            items: Vec::new(),
            imports: BTreeMap::new(),
            test: false,
        };
        let incomplete = BTreeMap::from([("crate".into(), module("crate", "src/lib.rs"))]);
        assert_eq!(
            validate_module_coverage(&incomplete, &parsed).unwrap_err(),
            "unreachable/cyclic module source omitted from traversal"
        );

        let complete = BTreeMap::from([
            ("crate".into(), module("crate", "src/lib.rs")),
            (
                "crate::orphan".into(),
                module("crate::orphan", "src/orphan.rs"),
            ),
        ]);
        validate_module_coverage(&complete, &parsed).unwrap();
    }

    #[test]
    fn cargo_target_roots_hold_their_own_module_declarations() {
        for root in [
            "tests/cli.rs",
            "examples/demo.rs",
            "benches/speed.rs",
            "build.rs",
            "crates/core/tests/cli.rs",
            "crates/core/build.rs",
            "src/bin/tool.rs",
        ] {
            assert!(cargo_target_root(root), "{root} is a target root");
        }
        for module in [
            "src/tests/cli.rs",
            "src/build.rs",
            "tests/common/mod.rs",
            "src/lib.rs",
        ] {
            assert!(!cargo_target_root(module), "{module} is a module");
        }
        assert_eq!(root_base("tests/cli.rs"), Path::new("tests"));
        assert_eq!(root_base("src/bin/tool.rs"), Path::new("src/bin"));
        assert_eq!(root_base("src/graph.rs"), Path::new("src/graph"));
    }

    #[test]
    fn duplicate_import_bindings_are_ambiguous_only_when_resolved() {
        let file = syn::parse_file(
            "use crate::extract::FromRef; use crate::macros::FromRef; use http; \
             use crate::Router; use crate::Router; use crate::Other as _; use crate::Skip as _; \
             use parse::parse; use serde as json;",
        )
        .unwrap();
        let mut bindings = BTreeMap::new();
        for item in &file.items {
            if let syn::Item::Use(item) = item {
                imports(&item.tree, &[], &mut bindings).unwrap();
            }
        }
        assert_eq!(
            imported_path(&bindings, "FromRef").unwrap_err(),
            "ambiguous import binding: FromRef"
        );
        assert_eq!(
            imported_path(&bindings, "Router").unwrap().unwrap(),
            &vec!["crate".to_owned(), "Router".to_owned()]
        );
        assert!(imported_path(&bindings, "_").unwrap().is_none());
        assert!(imported_path(&bindings, "Missing").unwrap().is_none());
        assert_eq!(
            imported_path(&bindings, "http").unwrap().unwrap(),
            &vec!["\0extern".to_owned(), "http".to_owned()]
        );
        assert_eq!(
            imported_path(&bindings, "parse").unwrap().unwrap(),
            &vec!["parse".to_owned(), "parse".to_owned()]
        );
        assert_eq!(
            imported_path(&bindings, "json").unwrap().unwrap(),
            &vec!["serde".to_owned()]
        );
    }

    #[test]
    fn modules_nested_in_cfg_test_modules_stay_test_code() {
        let parsed = BTreeMap::new();
        let mut result = BTreeMap::new();
        let mut walker = ModuleWalker {
            root: "crate",
            parsed: &parsed,
            stack: Vec::new(),
            result: &mut result,
            test: false,
        };
        let file = syn::parse_file("#[cfg(test)] mod tests { mod inner {} } mod live {}").unwrap();
        walker
            .visit(
                "src/lib.rs",
                "crate",
                &file.items,
                Path::new("src"),
                Path::new("src"),
            )
            .unwrap();
        let test = |key: &str| result[key].test;
        assert!(!test("crate"));
        assert!(test("crate::tests"));
        assert!(test("crate::tests::inner"));
        assert!(!test("crate::live"));
    }

    #[test]
    fn rust_parse_errors_name_the_file_line_and_column() {
        let Err(error) = syn::parse_file("fn ok() {}\nfn broken( {}\n") else {
            panic!("an unclosed parameter list must not parse");
        };
        let message = super::super::parse_error_message("src/lib.rs", &error);
        assert!(
            message.starts_with("parse error in src/lib.rs:2:"),
            "{message}"
        );
        assert!(message.ends_with(&format!(": {error}")), "{message}");
    }
}
