use super::*;

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
            found = authored_path_attribute(attr, found)?;
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
    found: Option<String>,
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
    let found: Vec<_> = candidates
        .into_iter()
        .filter(|p| parsed.contains_key(p))
        .collect();
    if found.len() != 1 {
        return Err(format!("missing or ambiguous module source: {name}"));
    }
    Ok(found[0].clone())
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
        UseTree::Rename(rename) => import_rename(rename, prefix, result),
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

fn insert_import(
    alias: String,
    path: Vec<String>,
    result: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    if result.insert(alias.clone(), path).is_some() {
        Err(format!("ambiguous import binding: {alias}"))
    } else {
        Ok(())
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
    insert_import(alias, path, result)
}

fn import_rename(
    rename: &UseRename,
    prefix: &[String],
    result: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    let mut path = prefix.to_vec();
    if rename.ident != "self" {
        path.push(rename.ident.to_string());
    }
    let alias = rename.rename.to_string();
    if alias == "_" {
        Ok(())
    } else {
        insert_import(alias, path, result)
    }
}

struct ModuleWalker<'a> {
    root: &'a str,
    parsed: &'a BTreeMap<String, syn::File>,
    stack: Vec<String>,
    result: &'a mut BTreeMap<String, Module>,
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
    pub fn owner(&self, ty: &Type, module: &str) -> Result<Option<String>, String> {
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
    pub fn resolve(
        &self,
        path: &[String],
        module: &str,
        depth: usize,
    ) -> Result<Option<String>, String> {
        self.resolve_bounded(path, module, depth, &mut 0)
    }

    fn super_prefix(
        &self,
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
            "super" => self.super_prefix(path, module, &context.root)?,
            _ => {
                if let Some(import) = context.imports.get(first) {
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
            if let Some(import) = self.modules.get(&parent).and_then(|m| m.imports.get(part)) {
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
    ) -> Result<Option<String>, String> {
        let leaf = remaining.last().ok_or("empty resolved type path")?;
        let mut matches = self.glob_matches(parent, leaf, depth, visits)?;
        matches.sort();
        matches.dedup();
        if matches.len() > 1 {
            return Err(format!(
                "ambiguous glob owner in {module}: {}",
                original.join("::")
            ));
        }
        Ok(matches.pop())
    }

    fn glob_matches(
        &self,
        parent: &str,
        leaf: &str,
        depth: usize,
        visits: &mut usize,
    ) -> Result<Vec<String>, String> {
        let mut matches = vec![];
        let Some(context) = self.modules.get(parent) else {
            return Ok(matches);
        };
        for (name, import) in &context.imports {
            if name.starts_with('*')
                && let Some(owner) = self.glob_match(import, leaf, parent, depth, visits)?
            {
                matches.push(owner);
            }
        }
        Ok(matches)
    }

    fn glob_match(
        &self,
        import: &[String],
        leaf: &str,
        parent: &str,
        depth: usize,
        visits: &mut usize,
    ) -> Result<Option<String>, String> {
        let mut candidate = import.to_vec();
        candidate.push(leaf.to_string());
        match self.resolve_bounded(&candidate, parent, depth + 1, visits) {
            Ok(owner) => Ok(owner),
            Err(error) if error.contains("cycle/budget") || error.starts_with("ambiguous") => {
                Err(error)
            }
            Err(_) => Ok(None),
        }
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
        let external = matches!(first, "std" | "core" | "alloc")
            || (path.len() > 1
                && !self.modules.contains_key(&format!("{module}::{first}"))
                && !matches!(first, "crate" | "self" | "super"));
        if primitive || external {
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
        Ok(
            match self.glob_owner(&parent, remaining, module, original, depth, visits)? {
                Some(owner) => OwnerLookup::Resolved(Some(owner)),
                None => OwnerLookup::Missing,
            },
        )
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
                let error_location = location(path, error.span());
                report.error_at(format!("parse error in {path}: {error}"), error_location);
            }
        }
    }
    parsed
}

pub(super) fn referenced_sources(
    parsed: &BTreeMap<String, syn::File>,
    report: &mut Report,
) -> BTreeSet<String> {
    let mut referenced = BTreeSet::new();
    for (path, file) in parsed {
        let attribute_base = Path::new(path).parent().unwrap_or(Path::new(""));
        if let Err(error) = references(
            &file.items,
            &file_base(path),
            attribute_base,
            parsed,
            &mut referenced,
            0,
        ) {
            report.error(format!("{path}: {error}"));
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
        };
        if let Err(error) = walker.visit(
            &root,
            &root,
            &parsed[&root].items,
            &file_base(&root),
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
    report: &mut Report,
) {
    let included: BTreeSet<_> = modules.values().map(|module| &module.path).collect();
    if included.len() != parsed.len() {
        report.error("unreachable/cyclic module source omitted from traversal");
    }
}
