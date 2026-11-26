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

/// The parsed corpus and the in-root directory symlinks a module path may
/// cross on its way to a captured file.
#[derive(Clone, Copy)]
pub(super) struct Sources<'a> {
    pub parsed: &'a BTreeMap<String, syn::File>,
    pub directory_links: &'a BTreeMap<String, String>,
}

impl Sources<'_> {
    /// The corpus file a module path names, following directory symlinks to
    /// the file captured at their target so it is scanned once. `None` when no
    /// captured file matches, including paths through links that leave the
    /// corpus root, which were never recorded.
    fn resolve(&self, path: String) -> Result<Option<String>, String> {
        let mut path = path;
        for _ in 0..=self.directory_links.len() {
            if self.parsed.contains_key(&path) {
                return Ok(Some(path));
            }
            let through_link = self.directory_links.iter().find_map(|(link, target)| {
                let rest = path.strip_prefix(link.as_str())?.strip_prefix('/')?;
                Some(Path::new(target).join(rest))
            });
            match through_link {
                Some(next) => path = clean_path(&next)?,
                None => return Ok(None),
            }
        }
        Err(format!("cyclic directory symlink: {path}"))
    }
}

/// The `#[path]` attributes of a module declaration: the unconditional one, if
/// any, and every one a `#[cfg_attr(…, path = "…")]` selects under some `cfg`.
#[derive(Debug, Default, PartialEq)]
struct PathAttributes {
    plain: Option<String>,
    conditional: Vec<String>,
}

fn path_attributes(item: &ItemMod) -> Result<PathAttributes, String> {
    let mut found = PathAttributes::default();
    for attr in &item.attrs {
        if attr.path().is_ident("path") {
            if found.plain.is_some() {
                return Err("ambiguous or non-portable module path attribute".into());
            }
            found.plain = Some(authored_path_attribute(&attr.meta)?);
        } else if conditional_path_attribute(attr) {
            found.conditional.extend(conditional_paths(attr)?);
        }
    }
    Ok(found)
}

fn conditional_path_attribute(attribute: &Attribute) -> bool {
    attribute.path().is_ident("cfg_attr")
        && matches!(&attribute.meta, Meta::List(list) if mentions_path(list.tokens.clone()))
}

/// The paths one `#[cfg_attr(predicate, attributes…)]` applies. A `path` nested
/// in another `cfg_attr` would need the predicates combined, so it is refused.
fn conditional_paths(attribute: &Attribute) -> Result<Vec<String>, String> {
    let attributes = attribute
        .parse_args_with(|input: syn::parse::ParseStream<'_>| {
            input.parse::<Meta>()?;
            input.parse::<syn::Token![,]>()?;
            syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated(input)
        })
        .map_err(|_| "unsupported conditional module path attribute")?;
    let mut paths = vec![];
    for meta in &attributes {
        if meta.path().is_ident("path") {
            paths.push(authored_path_attribute(meta)?);
        } else if meta.path().is_ident("cfg_attr") {
            return Err("nested conditional module path requires active-cfg analysis".into());
        }
    }
    Ok(paths)
}

fn authored_path_attribute(meta: &Meta) -> Result<String, String> {
    let Meta::NameValue(value) = meta else {
        return Err("unsupported module path attribute".into());
    };
    let Expr::Lit(ExprLit {
        lit: Lit::Str(path),
        ..
    }) = &value.value
    else {
        return Err("unsupported module path attribute".into());
    };
    if path.value().contains('\\') || Path::new(&path.value()).is_absolute() {
        return Err("ambiguous or non-portable module path attribute".into());
    }
    Ok(path.value())
}

fn mentions_path(tokens: proc_macro2::TokenStream) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(ident) => ident == "path",
        proc_macro2::TokenTree::Group(group) => mentions_path(group.stream()),
        _ => false,
    })
}

fn inline_base(item: &ItemMod, base: &Path, attribute_base: &Path) -> Result<PathBuf, String> {
    let paths = path_attributes(item)?;
    if !paths.conditional.is_empty() {
        return Err("conditional inline module path requires active-cfg analysis".into());
    }
    let next = match paths.plain {
        Some(path) => attribute_base.join(path),
        None => base.join(item.ident.to_string()),
    };
    Ok(PathBuf::from(clean_path(&next)?))
}

/// Every source file a file module declaration can load, one per
/// configuration variant in declaration order. Each explicit path must be in
/// the corpus. The conventional `name.rs` or `name/mod.rs` is required only
/// when no `cfg_attr` path offers an alternative.
fn module_files(
    item: &ItemMod,
    base: &Path,
    attribute_base: &Path,
    sources: Sources<'_>,
) -> Result<Vec<String>, String> {
    let paths = path_attributes(item)?;
    let mut files = vec![];
    for path in paths.conditional.iter().chain(&paths.plain) {
        push_unique(
            &mut files,
            attributed_module_file(path, attribute_base, sources)?,
        );
    }
    if paths.plain.is_some() {
        return Ok(files);
    }
    let name = item.ident.to_string();
    match conventional_module_file(&name, base, sources)? {
        Some(path) => push_unique(&mut files, path),
        None if !files.is_empty() => {}
        None => return Err(format!("missing or ambiguous module source: {name}")),
    }
    Ok(files)
}

fn push_unique(files: &mut Vec<String>, path: String) {
    if !files.contains(&path) {
        files.push(path);
    }
}

/// The corpus source a `#[path]` attribute names relative to `attribute_base`.
fn attributed_module_file(
    path: &str,
    attribute_base: &Path,
    sources: Sources<'_>,
) -> Result<String, String> {
    let path = clean_path(&attribute_base.join(path))?;
    sources
        .resolve(path.clone())?
        .ok_or_else(|| format!("module source missing from corpus: {path}"))
}

/// The one source file `mod {name};` names next to `base`, as `{name}.rs` or
/// `{name}/mod.rs`, or `None` when neither or both exist.
fn conventional_module_file(
    name: &str,
    base: &Path,
    sources: Sources<'_>,
) -> Result<Option<String>, String> {
    let candidates = [
        clean_path(&base.join(format!("{name}.rs")))?,
        clean_path(&base.join(name).join("mod.rs"))?,
    ];
    let mut found = vec![];
    for candidate in candidates {
        if let Some(path) = sources.resolve(candidate)? {
            found.push(path);
        }
    }
    Ok(match <[String; 1]>::try_from(found) {
        Ok([path]) => Some(path),
        Err(_) => None,
    })
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
    sources: Sources<'_>,
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
                references(items, &next, &next, sources, found, depth + 1)?;
            } else {
                found.extend(module_files(module, base, attribute_base, sources)?);
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

/// Heads the binding recorded for a name that two imports bring in, followed
/// by each imported path joined with `::`. Rust allows that when the items live
/// in different namespaces, such as `pub use self::from_ref::FromRef;` and
/// `pub use axum_macros::FromRef;`, or under exclusive `cfg`s, such as a mock
/// and a real type imported under one name.
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
            let joined = path.join("::");
            let binding = entry.get_mut();
            if binding
                .first()
                .is_some_and(|first| first == AMBIGUOUS_IMPORT)
            {
                if !binding.contains(&joined) {
                    binding.push(joined);
                }
            } else if *binding != path {
                *binding = vec![AMBIGUOUS_IMPORT.into(), binding.join("::"), joined];
            }
        }
        Entry::Vacant(entry) => {
            entry.insert(path);
        }
    }
}

/// Every path the imports bind `name` to, in import order.
fn imported_paths(imports: &BTreeMap<String, Vec<String>>, name: &str) -> Vec<Vec<String>> {
    match imports.get(name) {
        Some(binding)
            if binding
                .first()
                .is_some_and(|first| first == AMBIGUOUS_IMPORT) =>
        {
            binding[1..]
                .iter()
                .map(|path| path.split("::").map(str::to_string).collect())
                .collect()
        }
        binding => binding.into_iter().cloned().collect(),
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
    sources: Sources<'a>,
    stack: Vec<String>,
    result: &'a mut BTreeMap<String, Module>,
    variants: &'a mut BTreeMap<String, Vec<String>>,
    /// Inside a test target root or a `#[cfg(test)]` module.
    test: bool,
    /// Inside a file that an earlier module path already loaded: the key of
    /// that repeated load and of the first load.
    repeated: Option<(String, String)>,
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

    /// Records one module and returns its key. A module path declared again
    /// under another `cfg` is a configuration variant with its own key.
    fn insert(&mut self, path: &str, key: &str, items: &[Item]) -> Result<String, String> {
        let key = if self.result.contains_key(key) {
            let alternatives = self.variants.entry(key.into()).or_default();
            let variant = super::variant_key(key, alternatives.len() + 2);
            alternatives.push(variant.clone());
            variant
        } else {
            key.to_string()
        };
        let module = Module {
            key: key.clone(),
            root: self.root.into(),
            path: path.into(),
            items: items.to_vec(),
            imports: Self::bindings(items)?,
            test: self.test,
            repeated: self.repeated.as_ref().map(|(repeat, first)| {
                format!(
                    "{first}{}",
                    key.strip_prefix(repeat.as_str()).unwrap_or_default()
                )
            }),
        };
        self.result.insert(key.clone(), module);
        Ok(key)
    }

    /// The key of the module that first loaded the file at `path`, if any.
    fn first_load(&self, path: &str) -> Option<String> {
        self.result
            .values()
            .filter(|module| module.path == path && module.repeated.is_none())
            .map(|module| &module.key)
            .min_by_key(|key| key.len())
            .cloned()
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
        for next_path in module_files(module, base, attribute_base, self.sources)? {
            if self.stack.contains(&next_path) {
                return Err(format!("cyclic module source: {next_path}"));
            }
            let enclosing_repeated = self.repeated.clone();
            if self.repeated.is_none() {
                self.repeated = self
                    .first_load(&next_path)
                    .map(|first| (next_key.clone(), first));
            }
            self.stack.push(next_path.clone());
            let result = self.visit(
                &next_path,
                &next_key,
                &self.sources.parsed[&next_path].items,
                &file_base(&next_path),
                Path::new(&next_path).parent().unwrap_or(Path::new("")),
            );
            self.stack.pop();
            self.repeated = enclosing_repeated;
            result?;
        }
        Ok(())
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
        let key = self.insert(path, key, items)?;
        for item in items {
            if let Item::Mod(module) = item {
                self.visit_child(module, path, &key, base, attribute_base)?;
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
            // Traits own no impl blocks, so `impl dyn Trait` has no local
            // owner, and `!` is a primitive.
            Type::Array(_)
            | Type::Slice(_)
            | Type::Tuple(_)
            | Type::Reference(_)
            | Type::Ptr(_)
            | Type::TraitObject(_)
            | Type::Never(_) => Ok(None),
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
                if let OwnerLookup::Resolved(owner) = self.imported_name(
                    &context.imports,
                    first,
                    &path[1..],
                    module,
                    (depth, visits),
                )? {
                    return Ok(ResolutionStart::Resolved(owner));
                }
                (module.to_string(), 0)
            }
        };
        Ok(ResolutionStart::Continue {
            prefix,
            remaining: path[consumed..].to_vec(),
        })
    }

    /// The owner `name::rest` resolves to through the imports of `module`, or
    /// `Missing` when no import binds `name`. A name two imports bind resolves when
    /// any of them does, and is ambiguous when none does or they reach
    /// different local owners.
    fn imported_name(
        &self,
        imports: &BTreeMap<String, Vec<String>>,
        name: &str,
        rest: &[String],
        module: &str,
        (depth, visits): (usize, &mut usize),
    ) -> Result<OwnerLookup, String> {
        let paths = imported_paths(imports, name);
        let resolve = |path: &Vec<String>, visits: &mut usize| {
            let mut next = path.clone();
            next.extend_from_slice(rest);
            self.resolve_bounded(&next, module, depth + 1, visits)
        };
        if let [path] = paths.as_slice() {
            return resolve(path, visits).map(OwnerLookup::Resolved);
        }
        let mut owners = BTreeSet::new();
        let mut resolved = false;
        for path in &paths {
            match resolve(path, visits) {
                Ok(owner) => {
                    resolved = true;
                    owners.extend(owner);
                }
                Err(error) if error.contains("cycle/budget") => return Err(error),
                Err(_) => {}
            }
        }
        if paths.is_empty() {
            Ok(OwnerLookup::Missing)
        } else if resolved && owners.len() < 2 {
            Ok(OwnerLookup::Resolved(owners.pop_first()))
        } else {
            Err(format!("ambiguous import binding: {name}"))
        }
    }

    /// `key`, then the same path through each configuration variant of the
    /// modules it names, so a name resolves when any variant declares it.
    fn variant_keys(&self, key: &str) -> Vec<String> {
        let mut keys = vec![key.to_string()];
        for index in 0.. {
            let Some(current) = keys.get(index).cloned() else {
                break;
            };
            for (end, _) in current.match_indices("::") {
                let Some(alternatives) = self.variants.get(&current[..end]) else {
                    continue;
                };
                for alternative in alternatives {
                    let key = format!("{alternative}{}", &current[end..]);
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
        }
        keys
    }

    fn exact_owner(
        &self,
        key: &str,
        depth: usize,
        visits: &mut usize,
    ) -> Result<OwnerLookup, String> {
        let keys = self.variant_keys(key);
        if let Some(key) = keys.iter().find(|key| self.types.contains_key(*key)) {
            return Ok(OwnerLookup::Resolved(Some(key.clone())));
        }
        let Some((module, alias)) = keys.iter().find_map(|key| self.aliases.get(key)) else {
            return self.first_load_owner(key, depth, visits);
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

    /// Resolves a type of a repeated module load at the file's first load,
    /// which alone holds its declarations.
    fn first_load_owner(
        &self,
        key: &str,
        depth: usize,
        visits: &mut usize,
    ) -> Result<OwnerLookup, String> {
        let first = key.rsplit_once("::").and_then(|(module, name)| {
            let first = self.modules.get(module)?.repeated.as_ref()?;
            Some(format!("{first}::{name}"))
        });
        match first {
            Some(first) => self.exact_owner(&first, depth, visits),
            None => Ok(OwnerLookup::Missing),
        }
    }

    fn imported_owner(
        &self,
        prefix: &str,
        remaining: &[String],
        depth: usize,
        visits: &mut usize,
    ) -> Result<OwnerLookup, String> {
        let mut parent = prefix.to_string();
        for (index, part) in remaining.iter().enumerate() {
            for variant in self.variant_keys(&parent) {
                let Some(context) = self.modules.get(&variant) else {
                    continue;
                };
                let rest = &remaining[index + 1..];
                if let OwnerLookup::Resolved(owner) =
                    self.imported_name(&context.imports, part, rest, &variant, (depth, visits))?
                {
                    return Ok(OwnerLookup::Resolved(owner));
                }
            }
            if index + 1 < remaining.len() {
                parent = format!("{parent}::{part}");
            }
        }
        Ok(OwnerLookup::Missing)
    }

    /// Resolves the path through the glob imports of the deepest named module
    /// that has any, so `crate::loom::sync::Arc` reaches `sync` re-exported by
    /// `pub use self::std::*` in `crate::loom`. The first segment of a longer
    /// path never resolves through a glob of the starting module, which would
    /// re-enter `use loom::sync::*` through itself.
    fn glob_owner(
        &self,
        prefix: &str,
        remaining: &[String],
        module: &str,
        original: &[String],
        depth: usize,
        visits: &mut usize,
    ) -> Result<OwnerLookup, String> {
        if remaining.is_empty() {
            return Err("empty resolved type path".into());
        }
        let mut candidates = Vec::new();
        for split in (remaining.len().min(2) - 1..remaining.len()).rev() {
            let parent = std::iter::once(prefix)
                .chain(remaining[..split].iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("::");
            candidates = self.glob_matches(&parent, &remaining[split..], depth, visits)?;
            if !candidates.is_empty() {
                break;
            }
        }
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
        tail: &[String],
        depth: usize,
        visits: &mut usize,
    ) -> Result<Vec<Option<String>>, String> {
        let mut matches = vec![];
        for variant in self.variant_keys(parent) {
            let Some(context) = self.modules.get(&variant) else {
                continue;
            };
            for (name, import) in &context.imports {
                if name.starts_with('*') {
                    self.glob_match(
                        import,
                        tail,
                        (variant.as_str(), depth),
                        visits,
                        &mut matches,
                    )?;
                }
            }
        }
        Ok(matches)
    }

    /// Pushes the owner `tail` resolves to through one glob import, if it resolves.
    fn glob_match(
        &self,
        import: &[String],
        tail: &[String],
        (parent, depth): (&str, usize),
        visits: &mut usize,
        matches: &mut Vec<Option<String>>,
    ) -> Result<(), String> {
        let mut candidate = import.to_vec();
        candidate.extend_from_slice(tail);
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
        // An opaque item macro in the same module, such as mockall's `mock!`,
        // can declare a bare name this scan cannot expand.
        let generated = path.len() == 1
            && self.modules.get(module).is_some_and(|context| {
                context
                    .items
                    .iter()
                    .any(|item| matches!(item, Item::Macro(item) if item.ident.is_none()))
            });
        if primitive || prelude || external || generated {
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
        if let OwnerLookup::Resolved(owner) =
            self.imported_owner(prefix, remaining, depth, visits)?
        {
            return Ok(OwnerLookup::Resolved(owner));
        }
        self.glob_owner(prefix, remaining, module, original, depth, visits)
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

/// `items` with each item macro that wraps ordinary items, such as tokio's
/// `cfg_rt! { … }` or `pin_project! { … }`, replaced by the items it wraps.
/// Other macro bodies, including `macro_rules!` definitions, stay opaque.
fn expand_item_macros(items: Vec<Item>) -> Vec<Item> {
    let mut expanded = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Item::Macro(item) if item.ident.is_none() => {
                match syn::parse2::<syn::File>(item.mac.tokens.clone()) {
                    Ok(file) if !file.items.iter().any(|i| matches!(i, Item::Verbatim(_))) => {
                        expanded.extend(expand_item_macros(file.items));
                    }
                    _ => expanded.push(Item::Macro(item)),
                }
            }
            Item::Mod(mut module) => {
                if let Some((_, items)) = &mut module.content {
                    *items = expand_item_macros(std::mem::take(items));
                }
                expanded.push(Item::Mod(module));
            }
            item => expanded.push(item),
        }
    }
    expanded
}

pub(super) fn parse_sources(input: &Input, report: &mut Report) -> BTreeMap<String, syn::File> {
    let mut parsed = BTreeMap::new();
    for (path, source) in &input.files {
        match syn::parse_file(source) {
            Ok(mut file) => {
                file.items = expand_item_macros(std::mem::take(&mut file.items));
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
    sources: Sources<'_>,
) -> Result<BTreeSet<String>, String> {
    let mut found = BTreeSet::new();
    let attribute_base = Path::new(path).parent().unwrap_or(Path::new(""));
    references(&file.items, base, attribute_base, sources, &mut found, 0)?;
    Ok(found)
}

pub(super) fn referenced_sources(sources: Sources<'_>, report: &mut Report) -> BTreeSet<String> {
    // A Cargo target root such as `tests/cli.rs` resolves its modules next to
    // itself, unless another file declares it as a module after all.
    let mut found: BTreeMap<&str, Result<BTreeSet<String>, String>> = sources
        .parsed
        .iter()
        .map(|(path, file)| {
            let references = file_references(path, file, &root_base(path), sources);
            (path.as_str(), references)
        })
        .collect();
    let referenced: BTreeSet<String> = found
        .values()
        .filter_map(|references| references.as_ref().ok())
        .flatten()
        .cloned()
        .collect();
    for (path, file) in sources.parsed {
        if cargo_target_root(path) && referenced.contains(path) {
            found.insert(path, file_references(path, file, &file_base(path), sources));
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
    sources: Sources<'_>,
    report: &mut Report,
) -> (BTreeMap<String, Module>, BTreeMap<String, Vec<String>>) {
    let mut modules = BTreeMap::new();
    let mut variants = BTreeMap::new();
    for root in roots {
        let mut walker = ModuleWalker {
            root: &root,
            sources,
            stack: vec![root.clone()],
            result: &mut modules,
            variants: &mut variants,
            test: crate::core::test_suite::conventional_test_path(&root),
            repeated: None,
        };
        if let Err(error) = walker.visit(
            &root,
            &root,
            &sources.parsed[&root].items,
            &root_base(&root),
            Path::new(&root).parent().unwrap_or(Path::new("")),
        ) {
            report.error(error);
        }
    }
    (modules, variants)
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
        Facts, Module, ModuleWalker, PathAttributes, Sources, cargo_target_root, clean_path,
        conditional_path_attribute, consume_resolution_budget, expand_item_macros, imported_paths,
        imports, path_attributes, references, root_base, validate_module_coverage,
    };
    use crate::{report::Location, scan::TypeFacts};
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::Path,
    };

    static NO_LINKS: BTreeMap<String, String> = BTreeMap::new();

    fn sources(parsed: &BTreeMap<String, syn::File>) -> Sources<'_> {
        Sources {
            parsed,
            directory_links: &NO_LINKS,
        }
    }

    #[test]
    fn sources_resolve_through_directory_links_and_reject_cycles() {
        let parsed = BTreeMap::from([("common/mod.rs".into(), syn::parse_file("").unwrap())]);
        let links = BTreeMap::from([
            ("src/linked".to_string(), "common".to_string()),
            ("src/hop".to_string(), "src/linked".to_string()),
            ("src/a".to_string(), "src/b".to_string()),
            ("src/b".to_string(), "src/a".to_string()),
        ]);
        let sources = Sources {
            parsed: &parsed,
            directory_links: &links,
        };
        let resolve = |path: &str| sources.resolve(path.into());
        assert_eq!(resolve("common/mod.rs"), Ok(Some("common/mod.rs".into())));
        assert_eq!(
            resolve("src/linked/mod.rs"),
            Ok(Some("common/mod.rs".into()))
        );
        assert_eq!(resolve("src/hop/mod.rs"), Ok(Some("common/mod.rs".into())));
        assert_eq!(resolve("src/linkedx/mod.rs"), Ok(None));
        assert_eq!(resolve("src/linked/other.rs"), Ok(None));
        assert!(
            resolve("src/a/mod.rs")
                .unwrap_err()
                .starts_with("cyclic directory symlink")
        );
    }

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
    fn module_path_attributes_are_literal_unique_and_portable() {
        let explicit: syn::ItemMod = syn::parse_quote!(
            #[path = "custom/domain.rs"]
            mod domain;
        );
        assert_eq!(
            path_attributes(&explicit).unwrap(),
            PathAttributes {
                plain: Some("custom/domain.rs".into()),
                conditional: vec![],
            }
        );

        let duplicate: syn::ItemMod = syn::parse_quote!(
            #[path = "first.rs"]
            #[path = "second.rs"]
            mod domain;
        );
        assert!(
            path_attributes(&duplicate)
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
            syn::parse_quote!(
                #[cfg_attr(unix, path("dynamic.rs"))]
                mod domain;
            ),
            syn::parse_quote!(
                #[cfg_attr(path)]
                mod domain;
            ),
        ] {
            assert!(path_attributes(&item).is_err());
        }

        let conditional: syn::ItemMod = syn::parse_quote!(
            #[cfg_attr(unix, path = "unix.rs")]
            #[cfg_attr(windows, allow(dead_code), path = "windows.rs")]
            mod domain;
        );
        assert!(conditional_path_attribute(&conditional.attrs[0]));
        assert_eq!(
            path_attributes(&conditional).unwrap(),
            PathAttributes {
                plain: None,
                conditional: vec!["unix.rs".into(), "windows.rs".into()],
            }
        );

        let nested: syn::ItemMod = syn::parse_quote!(
            #[cfg_attr(unix, cfg_attr(test, path = "unix.rs"))]
            mod domain;
        );
        assert!(path_attributes(&nested).unwrap_err().contains("active-cfg"));

        let unrelated: syn::ItemMod = syn::parse_quote!(
            #[cfg_attr(unix, allow(dead_code))]
            mod domain;
        );
        assert!(!conditional_path_attribute(&unrelated.attrs[0]));
        assert_eq!(
            path_attributes(&unrelated).unwrap(),
            PathAttributes::default()
        );

        let nested_path_token: syn::ItemMod = syn::parse_quote!(
            #[cfg_attr(any(unix, path), allow(dead_code))]
            mod domain;
        );
        assert!(conditional_path_attribute(&nested_path_token.attrs[0]));
        assert_eq!(
            path_attributes(&nested_path_token).unwrap(),
            PathAttributes::default()
        );
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
                sources(&parsed),
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
                sources(&parsed),
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
                sources(&parsed),
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
        let mut variants = BTreeMap::new();
        let mut walker = ModuleWalker {
            root: "crate",
            sources: sources(&parsed),
            stack: vec!["path".into(); 129],
            result: &mut result,
            variants: &mut variants,
            test: false,
            repeated: None,
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
        let mut variants = BTreeMap::new();
        let mut walker = ModuleWalker {
            root: "crate",
            sources: sources(&parsed),
            stack: Vec::new(),
            result: &mut result,
            variants: &mut variants,
            test: false,
            repeated: None,
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
        let mut variants = BTreeMap::new();
        let mut walker = ModuleWalker {
            root: "crate",
            sources: sources(&parsed),
            stack: vec!["path".into(); 128],
            result: &mut result,
            variants: &mut variants,
            test: false,
            repeated: None,
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
            variants: BTreeMap::new(),
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
            syn::parse_quote!(dyn Port + '_),
            syn::parse_quote!(!),
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
            variants: BTreeMap::new(),
            modules: BTreeMap::from([(
                "crate::domain".into(),
                Module {
                    key: "crate::domain".into(),
                    root: "crate".into(),
                    path: "src/domain.rs".into(),
                    items: Vec::new(),
                    test: false,
                    repeated: None,
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
            variants: BTreeMap::new(),
            modules: BTreeMap::from([(
                "crate".into(),
                Module {
                    key: "crate".into(),
                    root: "crate".into(),
                    path: "src/lib.rs".into(),
                    items: Vec::new(),
                    imports: BTreeMap::new(),
                    test: false,
                    repeated: None,
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
            variants: BTreeMap::new(),
            modules: BTreeMap::from([(
                "crate".into(),
                Module {
                    key: "crate".into(),
                    root: "crate".into(),
                    path: "src/lib.rs".into(),
                    items: Vec::new(),
                    imports: BTreeMap::from([("Alias".into(), vec!["Alias".into()])]),
                    test: false,
                    repeated: None,
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
            repeated: None,
        };
        let aliases = Facts {
            variants: BTreeMap::new(),
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
            variants: BTreeMap::new(),
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
            repeated: None,
        };
        let empty = Facts {
            variants: BTreeMap::new(),
            modules: BTreeMap::from([("crate".into(), module(BTreeMap::new()))]),
            types: BTreeMap::new(),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        let mut missing = Vec::new();
        empty
            .glob_match(&[], &["Missing".into()], ("crate", 0), &mut 0, &mut missing)
            .unwrap();
        assert!(missing.is_empty());

        let cyclic = Facts {
            variants: BTreeMap::new(),
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
                    &["Alias".into()],
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
            variants: BTreeMap::new(),
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
                .glob_match(
                    &[],
                    &["Thing".into()],
                    ("crate", 0),
                    &mut 0,
                    &mut Vec::new()
                )
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
            repeated: None,
        };
        let facts = Facts {
            variants: BTreeMap::new(),
            modules: BTreeMap::from([
                ("crate".into(), module("crate")),
                ("crate::local".into(), module("crate::local")),
                ("crate::mocks".into(), {
                    let mut mocks = module("crate::mocks");
                    mocks.items =
                        syn::parse_file("macro_rules! named { () => {} } mock! { pub File {} }")
                            .unwrap()
                            .items;
                    mocks
                }),
                ("crate::named".into(), {
                    let mut named = module("crate::named");
                    named.items = syn::parse_file("macro_rules! named { () => {} }")
                        .unwrap()
                        .items;
                    named
                }),
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
        let mock = vec!["MockFile".into()];
        assert_eq!(
            facts
                .unresolved_owner("MockFile", &mock, "crate::mocks")
                .unwrap(),
            None
        );
        for (path, module) in [
            (mock.clone(), "crate::named"),
            (mock, "crate::local"),
            (vec!["crate".into(), "MockFile".into()], "crate::mocks"),
        ] {
            assert!(
                facts
                    .unresolved_owner(&path[0], &path, module)
                    .unwrap_err()
                    .contains("unresolved local impl owner")
            );
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
            repeated: None,
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
        let path = |parts: &[&str]| -> Vec<String> {
            parts.iter().map(|part| (*part).to_owned()).collect()
        };
        assert_eq!(
            imported_paths(&bindings, "FromRef"),
            vec![
                path(&["crate", "extract", "FromRef"]),
                path(&["crate", "macros", "FromRef"])
            ]
        );
        assert_eq!(
            imported_paths(&bindings, "Router"),
            vec![path(&["crate", "Router"])]
        );
        assert!(imported_paths(&bindings, "_").is_empty());
        assert!(imported_paths(&bindings, "Missing").is_empty());
        assert_eq!(
            imported_paths(&bindings, "http"),
            vec![path(&["\0extern", "http"])]
        );
        assert_eq!(
            imported_paths(&bindings, "parse"),
            vec![path(&["parse", "parse"])]
        );
        assert_eq!(imported_paths(&bindings, "json"), vec![path(&["serde"])]);

        let file = syn::parse_file("use a::A; use b::A; use a::A; use c::A;").unwrap();
        let mut bindings = BTreeMap::new();
        for item in &file.items {
            if let syn::Item::Use(item) = item {
                imports(&item.tree, &[], &mut bindings).unwrap();
            }
        }
        assert_eq!(
            imported_paths(&bindings, "A"),
            vec![path(&["a", "A"]), path(&["b", "A"]), path(&["c", "A"])]
        );
    }

    #[test]
    fn import_twins_resolve_when_one_binding_reaches_one_local_owner() {
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
        let file = syn::parse_file(concat!(
            "use crate::a::Mock; use crate::missing::Mock;",
            "use crate::a::Real; use std::sync::Real;",
            "use crate::a::Twin; use crate::b::Twin;",
            "use crate::gone::Lost; use crate::missing::Lost;",
        ))
        .unwrap();
        let mut bindings = BTreeMap::new();
        for item in &file.items {
            if let syn::Item::Use(item) = item {
                imports(&item.tree, &[], &mut bindings).unwrap();
            }
        }
        let facts = Facts {
            variants: BTreeMap::new(),
            modules: BTreeMap::from([(
                "crate".into(),
                Module {
                    key: "crate".into(),
                    root: "crate".into(),
                    path: "src/lib.rs".into(),
                    items: Vec::new(),
                    test: false,
                    repeated: None,
                    imports: bindings,
                },
            )]),
            types: BTreeMap::from([
                ("crate::a::Mock".into(), type_fact()),
                ("crate::a::Real".into(), type_fact()),
                ("crate::a::Twin".into(), type_fact()),
                ("crate::b::Twin".into(), type_fact()),
            ]),
            functions: Vec::new(),
            slots: Vec::new(),
            aliases: BTreeMap::new(),
        };
        let resolve = |name: &str| facts.resolve(&[name.to_owned()], "crate", 0);

        assert_eq!(resolve("Mock").unwrap().as_deref(), Some("crate::a::Mock"));
        assert_eq!(resolve("Real").unwrap().as_deref(), Some("crate::a::Real"));
        assert_eq!(
            resolve("Twin").unwrap_err(),
            "ambiguous import binding: Twin"
        );
        assert_eq!(
            resolve("Lost").unwrap_err(),
            "ambiguous import binding: Lost"
        );
    }

    #[test]
    fn glob_reexports_resolve_through_intermediate_modules() {
        let location = Location {
            path: "src/lib.rs".into(),
            line: 1,
            column: 1,
        };
        let module = |key: &str, imports| Module {
            key: key.into(),
            root: "crate".into(),
            path: "src/lib.rs".into(),
            items: Vec::new(),
            imports,
            test: false,
            repeated: None,
        };
        let facts = Facts {
            variants: BTreeMap::new(),
            modules: BTreeMap::from([
                ("crate".into(), module("crate", BTreeMap::new())),
                (
                    "crate::loom".into(),
                    module(
                        "crate::loom",
                        BTreeMap::from([("*0".into(), vec!["self".into(), "imp".into()])]),
                    ),
                ),
                (
                    "crate::loom::mocked".into(),
                    module(
                        "crate::loom::mocked",
                        BTreeMap::from([("*0".into(), vec!["loom".into(), "sync".into()])]),
                    ),
                ),
                (
                    "crate::loom::imp::sync".into(),
                    module(
                        "crate::loom::imp::sync",
                        BTreeMap::from([(
                            "Weak".into(),
                            vec!["std".into(), "sync".into(), "Weak".into()],
                        )]),
                    ),
                ),
            ]),
            types: BTreeMap::from([(
                "crate::loom::imp::sync::Arc".into(),
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
            aliases: BTreeMap::new(),
        };
        let resolve = |parts: &[&str]| {
            let path = parts
                .iter()
                .map(|part| (*part).to_owned())
                .collect::<Vec<_>>();
            facts.resolve(&path, "crate", 0)
        };

        assert_eq!(
            resolve(&["crate", "loom", "sync", "Arc"])
                .unwrap()
                .as_deref(),
            Some("crate::loom::imp::sync::Arc")
        );
        assert_eq!(resolve(&["crate", "loom", "sync", "Weak"]).unwrap(), None);
        let mut visits = 0;
        assert_eq!(
            facts
                .resolve_bounded(
                    &["loom".into(), "sync".into(), "Arc".into()],
                    "crate::loom::mocked",
                    0,
                    &mut visits
                )
                .unwrap(),
            None
        );
        assert_eq!(visits, 1);
        assert!(resolve(&["crate", "loom", "sync", "Missing"]).is_err());
    }

    #[test]
    fn item_macros_expand_to_their_ordinary_items_only() {
        use syn::Item;
        let file = syn::parse_file(concat!(
            "cfg_rt! { mod runtime; pub use runtime::Handle; impl Handle {} }",
            "mod inner { cfg_net! { struct Socket; } }",
            "macro_rules! keep { () => {} }",
            "opaque! { not valid rust }",
            "verbatim! { fn signature_only(); }",
            "fn after() {}",
        ))
        .unwrap();
        let items = expand_item_macros(file.items);
        let kinds = items
            .iter()
            .map(|item| match item {
                Item::Mod(module) => match &module.content {
                    Some((_, items)) => format!("mod {} {{{}}}", module.ident, items.len()),
                    None => format!("mod {}", module.ident),
                },
                Item::Use(_) => "use".to_owned(),
                Item::Impl(_) => "impl".to_owned(),
                Item::Macro(item) => format!(
                    "macro {}",
                    item.ident.as_ref().map_or_else(
                        || item.mac.path.segments[0].ident.to_string(),
                        ToString::to_string
                    )
                ),
                Item::Fn(function) => format!("fn {}", function.sig.ident),
                _ => "other".to_owned(),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            [
                "mod runtime",
                "use",
                "impl",
                "mod inner {1}",
                "macro keep",
                "macro opaque",
                "macro verbatim",
                "fn after"
            ]
        );
        let Item::Mod(inner) = &items[3] else {
            panic!("inline module fixture");
        };
        assert!(matches!(
            inner.content.as_ref().map(|(_, items)| &items[0]),
            Some(Item::Struct(_))
        ));
    }

    #[test]
    fn modules_nested_in_cfg_test_modules_stay_test_code() {
        let parsed = BTreeMap::new();
        let mut result = BTreeMap::new();
        let mut variants = BTreeMap::new();
        let mut walker = ModuleWalker {
            root: "crate",
            sources: sources(&parsed),
            stack: Vec::new(),
            result: &mut result,
            variants: &mut variants,
            test: false,
            repeated: None,
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
