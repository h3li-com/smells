use crate::{
    input::{Implementation, Input},
    policy::{Policy, SourceRoleDeclaration},
};
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use tree_sitter::{Node, Parser};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedSourceRole {
    pub role: String,
    pub framework: String,
    pub implementation_id: String,
    pub path: String,
    pub path_scope: String,
    pub source: String,
    pub confidence: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RoleContext {
    pub source_role: String,
    pub declaration_role: String,
    pub framework: String,
    pub implementation_id: String,
    pub source: String,
    pub confidence: String,
    pub path_scope: String,
    pub evidence: Vec<String>,
    pub disposition: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MigrationArtifact {
    pub role: String,
    pub framework: String,
    pub implementation_id: String,
    pub path: String,
    pub status: String,
    pub evidence: Vec<String>,
}

#[derive(Default)]
pub struct MigrationArtifactIndex {
    drizzle_directories: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceRoleCandidate {
    pub role: String,
    pub path: String,
    pub confidence: String,
    pub evidence: Vec<String>,
}

fn glob_regex(scope: &str) -> Result<Regex, String> {
    let relative = scope
        .strip_prefix('/')
        .ok_or_else(|| format!("source role path is not repository-root anchored: {scope}"))?;
    let mut expression = String::from("^");
    let characters = relative.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == '*' {
            if characters.get(index + 1) == Some(&'*') {
                if characters.get(index + 2) == Some(&'/') {
                    expression.push_str("(?:.*/)?");
                    index += 3;
                } else {
                    expression.push_str(".*");
                    index += 2;
                }
            } else {
                expression.push_str("[^/]*");
                index += 1;
            }
        } else {
            expression.push_str(&regex::escape(&characters[index].to_string()));
            index += 1;
        }
    }
    expression.push('$');
    Regex::new(&expression).map_err(|error| format!("invalid source role path {scope}: {error}"))
}

fn declared_role(
    declaration: &SourceRoleDeclaration,
    declaration_index: usize,
    implementation_id: &str,
    path_scope: &str,
    path: &str,
) -> ResolvedSourceRole {
    let mut evidence = vec![format!("policy.source_roles[{declaration_index}]")];
    evidence.extend(
        declaration
            .configuration
            .iter()
            .map(|path| format!("policy_configuration_path:{path}")),
    );
    ResolvedSourceRole {
        role: declaration.role.clone(),
        framework: declaration.framework.clone(),
        implementation_id: implementation_id.into(),
        path: path.into(),
        path_scope: path_scope.into(),
        source: "declared".into(),
        confidence: "explicit".into(),
        evidence,
    }
}

pub fn resolve_source_roles(
    policy: &Policy,
    files: &BTreeMap<String, String>,
    language: &str,
    implementations: &[Implementation],
) -> Result<Vec<ResolvedSourceRole>, String> {
    let mut resolved = BTreeMap::<String, ResolvedSourceRole>::new();
    for (declaration_index, declaration) in policy.source_roles.iter().enumerate() {
        resolve_declared_source_role(
            declaration,
            declaration_index,
            files,
            implementations,
            &mut resolved,
        )?;
    }
    resolve_inferred_source_roles(language, files, implementations, &mut resolved)?;
    Ok(resolved.into_values().collect())
}

fn resolve_declared_source_role(
    declaration: &SourceRoleDeclaration,
    declaration_index: usize,
    files: &BTreeMap<String, String>,
    implementations: &[Implementation],
    resolved: &mut BTreeMap<String, ResolvedSourceRole>,
) -> Result<(), String> {
    let mut declaration_implementation = None::<&str>;
    for scope in &declaration.paths {
        let matcher = glob_regex(scope)?;
        for path in files.keys().filter(|path| matcher.is_match(path)) {
            let implementation = implementation_for_source(implementations, path)
                .ok_or_else(|| format!("source role path has no Implementation owner: {path}"))?;
            if declaration_implementation.is_some_and(|previous| previous != implementation.id) {
                return Err(format!(
                    "source role declaration crosses Implementation boundaries: {} and {}",
                    declaration_implementation.expect("checked source role owner"),
                    implementation.id
                ));
            }
            declaration_implementation = Some(&implementation.id);
            let role = declared_role(
                declaration,
                declaration_index,
                &implementation.id,
                scope,
                path,
            );
            if let Some(previous) = resolved.insert(path.clone(), role.clone()) {
                return Err(format!(
                    "ambiguous source role resolution for {path}: {} ({}) and {} ({})",
                    previous.framework, previous.path_scope, role.framework, role.path_scope
                ));
            }
        }
    }
    Ok(())
}

fn resolve_inferred_source_roles(
    language: &str,
    files: &BTreeMap<String, String>,
    implementations: &[Implementation],
    resolved: &mut BTreeMap<String, ResolvedSourceRole>,
) -> Result<(), String> {
    for (path, source) in files {
        if resolved.contains_key(path) {
            continue;
        }
        let implementation = implementation_for_source(implementations, path)
            .ok_or_else(|| format!("source file has no Implementation owner: {path}"))?;
        if let Some(role) = inferred_source_role(language, path, source, files, implementation) {
            resolved.insert(path.clone(), role);
        }
    }
    Ok(())
}

fn implementation_for_source<'a>(
    implementations: &'a [Implementation],
    path: &str,
) -> Option<&'a Implementation> {
    implementations
        .iter()
        .find(|implementation| implementation.source_files.iter().any(|file| file == path))
}

fn implementation_for_artifact<'a>(
    implementations: &'a [Implementation],
    path: &str,
) -> Option<&'a Implementation> {
    implementations
        .iter()
        .filter(|implementation| {
            implementation
                .root
                .as_deref()
                .is_some_and(|root| root == "." || Path::new(path).starts_with(Path::new(root)))
        })
        .max_by_key(|implementation| {
            implementation
                .root
                .as_deref()
                .map_or(0, |root| Path::new(root).components().count())
        })
        .or_else(|| {
            implementations
                .iter()
                .find(|implementation| implementation.ownership == "unowned_source")
        })
}

fn matches(source: &str, expression: &'static OnceLock<Regex>, pattern: &str) -> bool {
    expression
        .get_or_init(|| Regex::new(pattern).expect("static migration detector regex"))
        .is_match(source)
}

fn code_only(path: &str, source: &str) -> Option<String> {
    crate::collectors::code_only(path, source).ok()
}

fn actual_imports(path: &str, source: &str) -> Option<Vec<String>> {
    crate::collectors::import_statements(path, source).ok()
}

fn imported_from(imports: &[String], package: &str) -> bool {
    let package = regex::escape(package);
    let expression = Regex::new(&format!(
        r#"(?s)\bfrom\s*['\"]{package}(?:/[^'\"]*)?['\"]|\brequire\(\s*['\"]{package}(?:/[^'\"]*)?['\"]\s*\)"#
    ))
    .expect("escaped package name produces a valid import regex");
    imports
        .iter()
        .any(|statement| expression.is_match(statement))
}

fn python_imports(imports: &[String], module: &str) -> bool {
    let module = regex::escape(module);
    let expression = Regex::new(&format!(
        r"(?m)^\s*(?:from\s+{module}(?:\.[A-Za-z_][A-Za-z0-9_]*)*\s+import\b|import\s+{module}\b)"
    ))
    .expect("escaped Python module produces a valid import regex");
    imports
        .iter()
        .any(|statement| expression.is_match(statement))
}

fn alembic_revision_graph(source: &str) -> bool {
    static REVISION: OnceLock<Regex> = OnceLock::new();
    static DOWN_REVISION: OnceLock<Regex> = OnceLock::new();
    matches(
        source,
        &REVISION,
        r"(?m)^[ \t]*revision(?:[ \t]*:[^=]+)?[ \t]*=",
    ) && matches(
        source,
        &DOWN_REVISION,
        r"(?m)^[ \t]*down_revision(?:[ \t]*:[^=]+)?[ \t]*=",
    )
}

fn inferred_alembic(
    path: &str,
    source: &str,
    implementation_id: &str,
) -> Option<ResolvedSourceRole> {
    static UPGRADE: OnceLock<Regex> = OnceLock::new();
    let code = code_only(path, source)?;
    let imports = actual_imports(path, source)?;
    let imported = python_imports(&imports, "alembic");
    let upgrade = matches(
        &code,
        &UPGRADE,
        r"(?m)^(?:async[ \t]+)?def[ \t]+upgrade[ \t]*\([ \t]*\)[ \t]*(?:->[ \t]*[^:\n]+)?[ \t]*:",
    );
    if !(imported && alembic_revision_graph(&code) && upgrade) {
        return None;
    }
    Some(ResolvedSourceRole {
        role: "database_migration".into(),
        framework: "alembic".into(),
        implementation_id: implementation_id.into(),
        path: path.into(),
        path_scope: format!("/{path}"),
        source: "inferred".into(),
        confidence: "high".into(),
        evidence: vec![
            "framework_import:alembic".into(),
            "revision_graph_metadata".into(),
            "canonical_entrypoint:upgrade".into(),
        ],
    })
}

fn inferred_django(
    path: &str,
    source: &str,
    implementation_id: &str,
) -> Option<ResolvedSourceRole> {
    static MIGRATION_CLASS: OnceLock<Regex> = OnceLock::new();
    let code = code_only(path, source)?;
    let imports = actual_imports(path, source)?;
    let imported = python_imports(&imports, "django.db");
    let migration_class = matches(
        &code,
        &MIGRATION_CLASS,
        r"(?m)^\s*class\s+Migration\s*\(\s*migrations\.Migration\s*\)\s*:",
    );
    if imported && migration_class && code.contains("dependencies") && code.contains("operations") {
        Some(inferred_role(
            "django",
            path,
            implementation_id,
            [
                "framework_import:django_migrations",
                "migration_contract:Migration_subclass",
                "declarative_members:dependencies_and_operations",
            ],
        ))
    } else {
        None
    }
}

fn inferred_role(
    framework: &str,
    path: &str,
    implementation_id: &str,
    evidence: impl IntoIterator<Item = &'static str>,
) -> ResolvedSourceRole {
    ResolvedSourceRole {
        role: "database_migration".into(),
        framework: framework.into(),
        implementation_id: implementation_id.into(),
        path: path.into(),
        path_scope: format!("/{path}"),
        source: "inferred".into(),
        confidence: "high".into(),
        evidence: evidence.into_iter().map(str::to_string).collect(),
    }
}

fn exported_typescript_function(source: &str, name: &str) -> bool {
    static UP: OnceLock<Regex> = OnceLock::new();
    static DOWN: OnceLock<Regex> = OnceLock::new();
    match name {
        "up" => matches(
            source,
            &UP,
            r"(?m)^\s*export\s+(?:async\s+)?(?:function\s+up\b|const\s+up\s*(?::[^=]+)?=)",
        ),
        "down" => matches(
            source,
            &DOWN,
            r"(?m)^\s*export\s+(?:async\s+)?(?:function\s+down\b|const\s+down\s*(?::[^=]+)?=)",
        ),
        _ => false,
    }
}

fn exported_typescript_parameters(source: &str, name: &str) -> Option<String> {
    let escaped = regex::escape(name);
    let declaration = Regex::new(&format!(
        r"(?m)^[ \t]*export[ \t]+(?:async[ \t]+)?function[ \t]+{escaped}[ \t]*\(([^)]*)\)"
    ))
    .expect("escaped TypeScript function name");
    if let Some(parameters) = declaration.captures(source) {
        return Some(parameters[1].trim().to_string());
    }
    Regex::new(&format!(
        r"(?m)^[ \t]*export[ \t]+const[ \t]+{escaped}[ \t]*(?::[^=]+)?=[ \t]*(?:async[ \t]*)?\(([^)]*)\)"
    ))
    .expect("escaped TypeScript const name")
    .captures(source)
    .map(|parameters| parameters[1].trim().to_string())
}

fn exported_typescript_annotation(source: &str, name: &str) -> Option<String> {
    Regex::new(&format!(
        r"(?m)^[ \t]*export[ \t]+const[ \t]+{}[ \t]*:[ \t]*([^=\n]+)[ \t]*=",
        regex::escape(name)
    ))
    .expect("escaped TypeScript const name")
    .captures(source)
    .map(|annotation| annotation[1].trim().to_string())
}

#[derive(Debug)]
struct TypescriptClassShape {
    name: String,
    header: String,
    methods: BTreeMap<String, String>,
}

fn collect_typescript_classes(
    node: Node<'_>,
    source: &str,
    code: &str,
    classes: &mut Vec<TypescriptClassShape>,
) {
    if matches!(
        node.kind(),
        "class" | "class_declaration" | "abstract_class_declaration"
    ) && let Some(body) = node.child_by_field_name("body")
    {
        let mut methods = BTreeMap::new();
        let mut cursor = body.walk();
        for member in body.named_children(&mut cursor) {
            if member.kind() != "method_definition" {
                continue;
            }
            if let Some(name) = member.child_by_field_name("name") {
                let method_name = name.utf8_text(source.as_bytes()).unwrap_or_default();
                methods.insert(
                    method_name.to_string(),
                    code[member.byte_range()].to_string(),
                );
            }
        }
        classes.push(TypescriptClassShape {
            name: node
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source.as_bytes()).ok())
                .unwrap_or("<anonymous>")
                .to_string(),
            header: code[node.start_byte()..body.start_byte()].to_string(),
            methods,
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_typescript_classes(child, source, code, classes);
    }
}

fn typescript_class_shapes(source: &str) -> Option<Vec<TypescriptClassShape>> {
    let code = code_only("migration.ts", source)?;
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .ok()?;
    let tree = crate::typescript_compat::parse(&mut parser, source, "migration.ts").ok()?;
    if tree.root_node().has_error() {
        return None;
    }
    let mut classes = Vec::new();
    collect_typescript_classes(tree.root_node(), source, &code, &mut classes);
    Some(classes)
}

fn typescript_imported_names(
    imports: &[String],
    package: &str,
    imported_name: &str,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let bindings_expression =
        Regex::new(r"(?s)\{([^}]*)\}").expect("static TypeScript import bindings regex");
    let default_expression =
        Regex::new(r"(?m)^\s*import\s+(?:type\s+)?([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:,|from\b)")
            .expect("static TypeScript default import regex");
    for statement in imports
        .iter()
        .filter(|statement| imported_from(std::slice::from_ref(statement), package))
    {
        if let Some(bindings) = bindings_expression.captures(statement) {
            for binding in bindings[1].split(',') {
                let words = binding
                    .split_whitespace()
                    .filter(|word| *word != "type")
                    .collect::<Vec<_>>();
                if words.first() == Some(&imported_name) {
                    names.insert(
                        words
                            .get(2)
                            .filter(|_| words.get(1) == Some(&"as"))
                            .copied()
                            .unwrap_or(imported_name)
                            .to_string(),
                    );
                }
            }
        }
        if imported_name == "Migration"
            && let Some(default) = default_expression.captures(statement)
        {
            names.insert(default[1].to_string());
        }
    }
    names
}

fn class_has_relation(
    class: &TypescriptClassShape,
    relationship: &str,
    imported_names: &BTreeSet<String>,
) -> bool {
    imported_names.iter().any(|name| {
        Regex::new(&format!(
            r"\b{}\b[^{{}}]*\b{}\b",
            regex::escape(relationship),
            regex::escape(name)
        ))
        .expect("escaped TypeScript migration relationship")
        .is_match(&class.header)
    })
}

fn class_has_declared_relation(class: &TypescriptClassShape, relationship: &str) -> bool {
    Regex::new(&format!(
        r"\b{}\b[ \t]+[A-Za-z_$][A-Za-z0-9_$]*\b",
        regex::escape(relationship)
    ))
    .expect("escaped TypeScript relationship")
    .is_match(&class.header)
}

fn class_has_framework_relation(
    class: &TypescriptClassShape,
    imports: &[String],
    package: &str,
    imported_name: &str,
    relationship: &str,
    allow_declared_alias: bool,
) -> bool {
    let imported_names = typescript_imported_names(imports, package, imported_name);
    class_has_relation(class, relationship, &imported_names)
        || (allow_declared_alias && class_has_declared_relation(class, relationship))
}

fn method_parameters(method: &str, name: &str) -> Option<String> {
    Regex::new(&format!(r"(?s)\b{}\s*\(([^)]*)\)", regex::escape(name)))
        .expect("escaped TypeScript method name")
        .captures(method)
        .map(|captures| captures[1].trim().to_string())
}

fn parameter_uses_import(
    parameters: &str,
    imports: &[String],
    package: &str,
    imported_name: &str,
) -> bool {
    typescript_imported_names(imports, package, imported_name)
        .iter()
        .any(|name| {
            Regex::new(&format!(r"\b{}\b", regex::escape(name)))
                .expect("escaped TypeScript parameter type")
                .is_match(parameters)
        })
}

fn parameter_is_exact_import(
    parameters: &str,
    imports: &[String],
    package: &str,
    imported_name: &str,
) -> bool {
    typescript_imported_names(imports, package, imported_name)
        .iter()
        .any(|name| {
            Regex::new(&format!(
                r"^[ \t]*[A-Za-z_$][A-Za-z0-9_$]*[ \t]*:[ \t]*{}[ \t]*$",
                regex::escape(name)
            ))
            .expect("escaped exact TypeScript parameter type")
            .is_match(parameters)
        })
}

fn typescript_export_parameter_matches(
    framework: &str,
    code: &str,
    imports: &[String],
    name: &str,
    declared: bool,
) -> bool {
    let Some(parameters) = exported_typescript_parameters(code, name) else {
        return false;
    };
    if declared {
        return !parameters.is_empty();
    }
    package_bound_parameter(framework, &parameters, imports)
        || (framework == "sequelize_umzug" && umzug_migration_annotation(code, imports, name))
}

fn package_bound_parameter(framework: &str, parameters: &str, imports: &[String]) -> bool {
    match framework {
        "knex" => parameter_uses_import(parameters, imports, "knex", "Knex"),
        "kysely" => parameter_uses_import(parameters, imports, "kysely", "Kysely"),
        "sequelize_umzug" => {
            parameter_uses_import(parameters, imports, "sequelize", "QueryInterface")
        }
        _ => false,
    }
}

fn umzug_migration_annotation(code: &str, imports: &[String], name: &str) -> bool {
    let Some(annotation) = exported_typescript_annotation(code, name) else {
        return false;
    };
    typescript_imported_names(imports, "umzug", "Migration")
        .iter()
        .any(|imported| {
            Regex::new(&format!(r"\b{}\b", regex::escape(imported)))
                .expect("escaped Umzug migration type")
                .is_match(&annotation)
        })
}

fn code_constructs_import(
    code: &str,
    imports: &[String],
    package: &str,
    imported_name: &str,
) -> bool {
    typescript_imported_names(imports, package, imported_name)
        .iter()
        .any(|imported| {
            Regex::new(&format!(r"\bnew[ \t]+{}\b", regex::escape(imported)))
                .expect("escaped TypeScript import binding")
                .is_match(code)
        })
}

fn inferred_typeorm(imports: &[String], classes: &[TypescriptClassShape], source: &str) -> bool {
    imported_from(imports, "typeorm")
        && classes
            .iter()
            .any(|class| class_shell_matches("typeorm", source, &class.name, false))
}

fn inferred_mikro_orm(imports: &[String], classes: &[TypescriptClassShape], source: &str) -> bool {
    imported_from(imports, "@mikro-orm/migrations")
        && classes
            .iter()
            .any(|class| class_shell_matches("mikro_orm", source, &class.name, false))
}

fn inferred_kysely(imports: &[String], code: &str) -> bool {
    imported_from(imports, "kysely")
        && exported_typescript_function(code, "up")
        && typescript_export_parameter_matches("kysely", code, imports, "up", false)
}

fn inferred_knex(imports: &[String], code: &str) -> bool {
    imported_from(imports, "knex")
        && exported_typescript_function(code, "up")
        && exported_typescript_function(code, "down")
        && typescript_export_parameter_matches("knex", code, imports, "up", false)
        && typescript_export_parameter_matches("knex", code, imports, "down", false)
}

fn sequelize_entrypoints_use_query_interface(imports: &[String], code: &str) -> bool {
    ["up", "down"].iter().all(|name| {
        exported_typescript_parameters(code, name).is_some_and(|parameters| {
            package_bound_parameter("sequelize_umzug", &parameters, imports)
        })
    })
}

fn typed_umzug_has_database_context(imports: &[String], code: &str) -> bool {
    if !(umzug_migration_annotation(code, imports, "up")
        && umzug_migration_annotation(code, imports, "down"))
    {
        return false;
    }
    let typed_query_interface = ["up", "down"].iter().all(|name| {
        exported_typescript_annotation(code, name).is_some_and(|annotation| {
            parameter_uses_import(&annotation, imports, "sequelize", "QueryInterface")
        })
    });
    typed_query_interface || code_constructs_import(code, imports, "umzug", "SequelizeStorage")
}

fn inferred_sequelize_umzug(imports: &[String], code: &str) -> Option<&'static str> {
    let sequelize_context = imported_from(imports, "sequelize")
        && !typescript_imported_names(imports, "sequelize", "QueryInterface").is_empty();
    if !(imported_from(imports, "umzug") || sequelize_context) {
        return None;
    }
    if !(exported_typescript_function(code, "up") && exported_typescript_function(code, "down")) {
        return None;
    }
    if !(sequelize_entrypoints_use_query_interface(imports, code)
        || typed_umzug_has_database_context(imports, code))
    {
        return None;
    }
    Some(if sequelize_context {
        "framework_import:sequelize"
    } else {
        "framework_import:umzug"
    })
}

fn inferred_typescript(
    path: &str,
    source: &str,
    implementation_id: &str,
) -> Option<ResolvedSourceRole> {
    let code = code_only(path, source)?;
    let imports = actual_imports(path, source)?;
    let classes = typescript_class_shapes(source)?;
    if inferred_typeorm(&imports, &classes, source) {
        return Some(inferred_role(
            "typeorm",
            path,
            implementation_id,
            [
                "framework_import:typeorm",
                "migration_contract:MigrationInterface",
                "canonical_methods:up_and_down",
            ],
        ));
    }
    if inferred_mikro_orm(&imports, &classes, source) {
        return Some(inferred_role(
            "mikro_orm",
            path,
            implementation_id,
            [
                "framework_import:mikro_orm_migrations",
                "migration_contract:extends_Migration",
                "canonical_method:up_optional_down",
            ],
        ));
    }
    if inferred_kysely(&imports, &code) {
        return Some(inferred_role(
            "kysely",
            path,
            implementation_id,
            [
                "framework_import:kysely",
                "migration_contract:up_optional_down",
                "canonical_export:up",
            ],
        ));
    }
    if inferred_knex(&imports, &code) {
        return Some(inferred_role(
            "knex",
            path,
            implementation_id,
            [
                "framework_import:knex",
                "migration_contract:up_and_down",
                "canonical_export:up",
            ],
        ));
    }
    if let Some(framework_import) = inferred_sequelize_umzug(&imports, &code) {
        return Some(inferred_role(
            "sequelize_umzug",
            path,
            implementation_id,
            [
                framework_import,
                "database_context:sequelize",
                "migration_contract:up_and_down",
            ],
        ));
    }
    None
}

fn implementation_relative_path<'a>(implementation: &Implementation, path: &'a str) -> &'a str {
    implementation.root.as_deref().map_or(path, |root| {
        if root == "." {
            path
        } else {
            path.strip_prefix(&format!("{root}/")).unwrap_or(path)
        }
    })
}

fn refinery_embeds_path(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
    target_path: &str,
) -> bool {
    let target = implementation_relative_path(implementation, target_path);
    let explicit = Regex::new(r#"embed_migrations!\s*\(\s*\"([^\"\\]+)\"\s*\)"#)
        .expect("static Refinery embedding regex");
    let default = Regex::new(r"embed_migrations!\s*\(\s*\)")
        .expect("static default Refinery embedding regex");
    implementation.source_files.iter().any(|source_path| {
        files.get(source_path).is_some_and(|source| {
            let Some(code) = code_only(source_path, source) else {
                return false;
            };
            if !code.contains("refinery") || !code.contains("embed_migrations!") {
                return false;
            }
            let explicit_match = explicit.captures_iter(source).any(|capture| {
                let matched = capture.get(0).expect("Refinery embedding match");
                if !code[matched.range()].contains("embed_migrations!") {
                    return false;
                }
                let directory = capture[1].trim_start_matches("./");
                !directory.split('/').any(|component| component == "..")
                    && Path::new(target).starts_with(directory)
            });
            explicit_match
                || (default.is_match(&code) && Path::new(target).starts_with("migrations"))
        })
    })
}

fn embedded_directory_matches(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
    target_path: &str,
    source_marker: &str,
    explicit: &Regex,
    default: &Regex,
    default_directory: &str,
) -> bool {
    let target = implementation_relative_path(implementation, target_path);
    implementation.source_files.iter().any(|source_path| {
        files.get(source_path).is_some_and(|source| {
            let Some(code) = code_only(source_path, source) else {
                return false;
            };
            if !code.contains(source_marker) {
                return false;
            }
            explicit.captures_iter(source).any(|capture| {
                let matched = capture.get(0).expect("migration embedding match");
                if !code[matched.range()].contains('!') {
                    return false;
                }
                let directory = capture[1].trim_start_matches("./");
                !directory.split('/').any(|component| component == "..")
                    && Path::new(target).starts_with(directory)
            }) || (default.is_match(&code) && Path::new(target).starts_with(default_directory))
        })
    })
}

fn diesel_embeds_path(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
    target_path: &str,
) -> bool {
    static EXPLICIT: OnceLock<Regex> = OnceLock::new();
    static DEFAULT: OnceLock<Regex> = OnceLock::new();
    embedded_directory_matches(
        files,
        implementation,
        target_path,
        "diesel_migrations",
        EXPLICIT.get_or_init(|| {
            Regex::new(r#"embed_migrations!\s*\(\s*"([^"\\]+)"\s*\)"#)
                .expect("static Diesel embedding regex")
        }),
        DEFAULT.get_or_init(|| {
            Regex::new(r"embed_migrations!\s*\(\s*\)")
                .expect("static default Diesel embedding regex")
        }),
        "migrations",
    )
}

fn sqlx_embeds_path(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
    target_path: &str,
) -> bool {
    static EXPLICIT: OnceLock<Regex> = OnceLock::new();
    static DEFAULT: OnceLock<Regex> = OnceLock::new();
    embedded_directory_matches(
        files,
        implementation,
        target_path,
        "sqlx::migrate!",
        EXPLICIT.get_or_init(|| {
            Regex::new(r#"sqlx::migrate!\s*\(\s*"([^"\\]+)"\s*\)"#)
                .expect("static SQLx embedding regex")
        }),
        DEFAULT.get_or_init(|| {
            Regex::new(r"sqlx::migrate!\s*\(\s*\)").expect("static default SQLx embedding regex")
        }),
        "migrations",
    )
}

fn sea_orm_registry_contains(code: &str, module_name: &str) -> bool {
    if !code.contains("impl MigratorTrait") || !code.contains("fn migrations") {
        return false;
    }
    let registries = Regex::new(
        r"(?s)impl\s+MigratorTrait\s+for\s+[A-Za-z_][A-Za-z0-9_]*[^{}]*\{.*?fn\s+migrations\s*\([^)]*\)[^{]*\{.*?vec!\s*\[(.*?)\]",
    )
    .expect("static SeaORM registry regex");
    let member = Regex::new(&format!(
        r"\b{}\s*::\s*Migration\b",
        regex::escape(module_name)
    ))
    .expect("escaped SeaORM migration module name");
    registries
        .captures_iter(code)
        .any(|registry| member.is_match(&registry[1]))
}

fn sea_orm_module_path(implementation: &Implementation, path: &str) -> Option<String> {
    let relative = implementation_relative_path(implementation, path)
        .strip_prefix("src/")
        .unwrap_or_else(|| implementation_relative_path(implementation, path));
    let module = relative.strip_suffix(".rs")?;
    let module = module.strip_suffix("/mod").unwrap_or(module);
    (!module.is_empty() && !matches!(module, "lib" | "main")).then(|| module.replace('/', "::"))
}

fn unique_sea_orm_migration_type(code: &str) -> bool {
    static MIGRATION_TYPE: OnceLock<Regex> = OnceLock::new();
    MIGRATION_TYPE
        .get_or_init(|| {
            Regex::new(r"\b(?:struct|enum)\s+Migration\b")
                .expect("static SeaORM Migration type regex")
        })
        .find_iter(code)
        .count()
        == 1
}

fn inferred_rust(
    path: &str,
    source: &str,
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> Option<ResolvedSourceRole> {
    static REFINERY_FILE: OnceLock<Regex> = OnceLock::new();
    static REFINERY_SIGNATURE: OnceLock<Regex> = OnceLock::new();
    let code = code_only(path, source)?;
    let versioned_refinery_file = matches(path, &REFINERY_FILE, r"(?:^|/)[UV][0-9]+__[^/]+\.rs$");
    let refinery_signature = matches(
        &code,
        &REFINERY_SIGNATURE,
        r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+migration\s*\(\s*\)\s*->\s*String\b",
    );
    let embedded = refinery_embeds_path(files, implementation, path);
    if versioned_refinery_file && refinery_signature && embedded {
        return Some(inferred_role(
            "refinery",
            path,
            &implementation.id,
            [
                "versioned_filename:refinery",
                "canonical_signature:migration_to_string",
                "embedding_call:embed_migrations",
            ],
        ));
    }
    let module_name = sea_orm_module_path(implementation, path)?;
    let sea_orm_registered = implementation.source_files.iter().any(|path| {
        files.get(path).is_some_and(|candidate| {
            code_only(path, candidate)
                .is_some_and(|code| sea_orm_registry_contains(&code, &module_name))
        })
    });
    if code.contains("sea_orm_migration")
        && code.contains("DeriveMigrationName")
        && code.contains("impl MigrationTrait for Migration")
        && sea_orm_registered
    {
        return Some(inferred_role(
            "sea_orm",
            path,
            &implementation.id,
            [
                "framework_import:sea_orm_migration",
                "migration_contract:MigrationTrait",
                "migration_identity:DeriveMigrationName",
                "registry_membership:MigratorTrait",
            ],
        ));
    }
    None
}

fn inferred_source_role(
    language: &str,
    path: &str,
    source: &str,
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> Option<ResolvedSourceRole> {
    match language {
        "python" => inferred_alembic(path, source, &implementation.id)
            .or_else(|| inferred_django(path, source, &implementation.id)),
        "typescript" => inferred_typescript(path, source, &implementation.id),
        "rust" => inferred_rust(path, source, files, implementation),
        _ => None,
    }
}

fn leaf_symbol(symbol: &str) -> &str {
    symbol
        .rsplit([':', '.'])
        .find(|component| !component.is_empty())
        .unwrap_or(symbol)
}

fn declaration_role(framework: &str, symbol: &str) -> Option<&'static str> {
    let name = leaf_symbol(symbol);
    match (framework, name) {
        ("alembic", "upgrade")
        | ("typeorm", "up")
        | ("knex", "up")
        | ("mikro_orm", "up")
        | ("kysely", "up")
        | ("sequelize_umzug", "up") => Some("migration_up"),
        ("alembic", "downgrade")
        | ("typeorm", "down")
        | ("knex", "down")
        | ("mikro_orm", "down")
        | ("kysely", "down")
        | ("sequelize_umzug", "down") => Some("migration_down"),
        ("prisma_8", "operations") => Some("migration_operations"),
        ("refinery", "migration") => Some("migration_sql_builder"),
        _ => None,
    }
}

fn typeorm_class_shell(
    class: &TypescriptClassShape,
    imports: &[String],
    allow_declared_alias: bool,
) -> bool {
    let relation = class_has_framework_relation(
        class,
        imports,
        "typeorm",
        "MigrationInterface",
        "implements",
        allow_declared_alias,
    );
    let Some(up_parameters) = class
        .methods
        .get("up")
        .and_then(|method| method_parameters(method, "up"))
    else {
        return false;
    };
    let Some(down_parameters) = class
        .methods
        .get("down")
        .and_then(|method| method_parameters(method, "down"))
    else {
        return false;
    };
    relation
        && if allow_declared_alias {
            !up_parameters.is_empty() && !down_parameters.is_empty()
        } else {
            parameter_is_exact_import(&up_parameters, imports, "typeorm", "QueryRunner")
                && parameter_is_exact_import(&down_parameters, imports, "typeorm", "QueryRunner")
        }
}

fn mikro_orm_class_shell(
    class: &TypescriptClassShape,
    imports: &[String],
    allow_declared_alias: bool,
) -> bool {
    class_has_framework_relation(
        class,
        imports,
        "@mikro-orm/migrations",
        "Migration",
        "extends",
        allow_declared_alias,
    ) && class.methods.contains_key("up")
}

fn prisma_class_shell(
    class: &TypescriptClassShape,
    imports: &[String],
    allow_declared_alias: bool,
) -> bool {
    class_has_framework_relation(
        class,
        imports,
        "@prisma/migrations",
        "Migration",
        "extends",
        allow_declared_alias,
    ) && class.methods.get("operations").is_some_and(|method| {
        Regex::new(r"\bget\s+operations\s*\(")
            .expect("static Prisma operations getter regex")
            .is_match(method)
    })
}

fn class_shell_matches(
    framework: &str,
    source: &str,
    symbol: &str,
    allow_declared_alias: bool,
) -> bool {
    let imports = actual_imports("migration.ts", source).unwrap_or_default();
    let classes = typescript_class_shapes(source).unwrap_or_default();
    let matches = |class: &TypescriptClassShape| match framework {
        "typeorm" => typeorm_class_shell(class, &imports, allow_declared_alias),
        "mikro_orm" => mikro_orm_class_shell(class, &imports, allow_declared_alias),
        "prisma_8" => prisma_class_shell(class, &imports, allow_declared_alias),
        _ => false,
    };
    if allow_declared_alias && classes.iter().filter(|class| matches(class)).count() != 1 {
        return false;
    }
    classes
        .iter()
        .find(|class| class.name == symbol)
        .is_some_and(matches)
}

fn canonical_alembic(source: &str, name: &str, owner: Option<&str>) -> bool {
    let Some(code) = code_only("migration.py", source) else {
        return false;
    };
    owner.is_none()
        && alembic_revision_graph(&code)
        && Regex::new(&format!(
            r"(?m)^(?:async[ \t]+)?def[ \t]+{}[ \t]*\([ \t]*\)[ \t]*(?:->[ \t]*[^:\n]+)?[ \t]*:",
            regex::escape(name)
        ))
        .expect("escaped Python function name")
        .is_match(&code)
}

fn canonical_typescript_class(
    framework: &str,
    source: &str,
    owner: Option<&str>,
    declared: bool,
) -> bool {
    owner.is_some_and(|owner| class_shell_matches(framework, source, leaf_symbol(owner), declared))
}

fn canonical_typescript_export(
    framework: &str,
    source: &str,
    name: &str,
    owner: Option<&str>,
    declared: bool,
) -> bool {
    let Some(code) = code_only("migration.ts", source) else {
        return false;
    };
    let Some(imports) = actual_imports("migration.ts", source) else {
        return false;
    };
    owner.is_none()
        && exported_typescript_function(&code, name)
        && typescript_export_parameter_matches(framework, &code, &imports, name, declared)
}

fn canonical_refinery(source: &str, name: &str, owner: Option<&str>) -> bool {
    static REFINERY_SIGNATURE: OnceLock<Regex> = OnceLock::new();
    let Some(code) = code_only("migration.rs", source) else {
        return false;
    };
    owner.is_none()
        && name == "migration"
        && matches(
            &code,
            &REFINERY_SIGNATURE,
            r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+migration\s*\(\s*\)\s*->\s*String\b",
        )
}

fn canonical_function(
    framework: &str,
    source: &str,
    symbol: &str,
    owner: Option<&str>,
    declared: bool,
) -> bool {
    let name = leaf_symbol(symbol);
    match framework {
        "alembic" => canonical_alembic(source, name, owner),
        "typeorm" | "mikro_orm" | "prisma_8" => {
            canonical_typescript_class(framework, source, owner, declared)
        }
        "knex" | "kysely" | "sequelize_umzug" => {
            canonical_typescript_export(framework, source, name, owner, declared)
        }
        "refinery" => canonical_refinery(source, name, owner),
        _ => false,
    }
}

pub fn function_role(
    input: &Input,
    path: &str,
    symbol: &str,
    owner: Option<&str>,
) -> Option<RoleContext> {
    let role = input.source_roles.iter().find(|role| role.path == path)?;
    let source = input.files.get(path)?;
    let declaration_role = declaration_role(&role.framework, symbol)?;
    if !canonical_function(
        &role.framework,
        source,
        symbol,
        owner,
        role.source == "declared",
    ) {
        return None;
    }
    Some(RoleContext {
        source_role: role.role.clone(),
        declaration_role: declaration_role.into(),
        framework: role.framework.clone(),
        implementation_id: role.implementation_id.clone(),
        source: role.source.clone(),
        confidence: role.confidence.clone(),
        path_scope: role.path_scope.clone(),
        evidence: role.evidence.clone(),
        disposition: "reported_by_role".into(),
    })
}

pub fn class_role(input: &Input, path: &str, symbol: &str) -> Option<RoleContext> {
    let source_role = input.source_roles.iter().find(|role| role.path == path)?;
    let source = input.files.get(path)?;
    if !class_shell_matches(
        &source_role.framework,
        source,
        symbol,
        source_role.source == "declared",
    ) {
        return None;
    }
    Some(RoleContext {
        source_role: source_role.role.clone(),
        declaration_role: "migration_shell".into(),
        framework: source_role.framework.clone(),
        implementation_id: source_role.implementation_id.clone(),
        source: source_role.source.clone(),
        confidence: source_role.confidence.clone(),
        path_scope: source_role.path_scope.clone(),
        evidence: source_role.evidence.clone(),
        disposition: "reported_by_role".into(),
    })
}

pub fn type_role(input: &Input, path: &str, symbol: &str) -> Option<RoleContext> {
    let source_role = input.source_roles.iter().find(|role| role.path == path)?;
    if source_role.framework != "sea_orm" || leaf_symbol(symbol) != "Migration" {
        return None;
    }
    let source = input.files.get(path)?;
    let code = code_only(path, source)?;
    if !code.contains("impl MigrationTrait for Migration") || !unique_sea_orm_migration_type(&code)
    {
        return None;
    }
    Some(RoleContext {
        source_role: source_role.role.clone(),
        declaration_role: "migration_shell".into(),
        framework: source_role.framework.clone(),
        implementation_id: source_role.implementation_id.clone(),
        source: source_role.source.clone(),
        confidence: source_role.confidence.clone(),
        path_scope: source_role.path_scope.clone(),
        evidence: source_role.evidence.clone(),
        disposition: "reported_by_role".into(),
    })
}

fn artifact(
    framework: &str,
    implementation_id: &str,
    path: &str,
    evidence: impl IntoIterator<Item = &'static str>,
) -> MigrationArtifact {
    MigrationArtifact {
        role: "database_migration_sql".into(),
        framework: framework.into(),
        implementation_id: implementation_id.into(),
        path: path.into(),
        status: "migration_body_not_analyzed".into(),
        evidence: evidence.into_iter().map(str::to_string).collect(),
    }
}

#[derive(Debug)]
enum DrizzleProperty {
    Missing,
    Literal(String),
    Dynamic,
}

#[derive(Default)]
struct DrizzleConfiguration {
    directories: BTreeSet<String>,
}

fn drizzle_literal_directory(value: Node<'_>, source: &str) -> Option<String> {
    if value.kind() != "string" {
        return None;
    }
    let text = value.utf8_text(source.as_bytes()).ok()?;
    let path = text
        .strip_prefix(['\'', '"'])?
        .strip_suffix(['\'', '"'])?
        .trim_start_matches("./");
    (!path.is_empty()
        && !path.contains('\\')
        && !path.split('/').any(|component| component == ".."))
    .then(|| path.to_string())
}

fn drizzle_pair_property(member: Node<'_>, source: &str, name: &str) -> Option<DrizzleProperty> {
    if member.kind() != "pair" {
        return None;
    }
    let key = member
        .child_by_field_name("key")?
        .utf8_text(source.as_bytes())
        .ok()?
        .trim_matches(['\'', '"']);
    if key != name {
        return None;
    }
    Some(
        member
            .child_by_field_name("value")
            .and_then(|value| drizzle_literal_directory(value, source))
            .map_or(DrizzleProperty::Dynamic, DrizzleProperty::Literal),
    )
}

fn drizzle_property(object: Node<'_>, source: &str, name: &str) -> DrizzleProperty {
    let mut found = None;
    let mut cursor = object.walk();
    for member in object.named_children(&mut cursor) {
        if member.kind() == "spread_element" {
            return DrizzleProperty::Dynamic;
        }
        let Some(property) = drizzle_pair_property(member, source, name) else {
            continue;
        };
        if found.replace(property).is_some() {
            return DrizzleProperty::Dynamic;
        }
    }
    found.unwrap_or(DrizzleProperty::Missing)
}

fn record_drizzle_call(
    node: Node<'_>,
    source: &str,
    property: &str,
    argument_index: usize,
    missing_directory: Option<&str>,
    configuration: &mut DrizzleConfiguration,
) {
    let Some(arguments) = node
        .child_by_field_name("arguments")
        .or_else(|| node.named_child(1))
    else {
        return;
    };
    let mut cursor = arguments.walk();
    let Some(object) = arguments.named_children(&mut cursor).nth(argument_index) else {
        return;
    };
    if object.kind() != "object" {
        return;
    }
    match drizzle_property(object, source, property) {
        DrizzleProperty::Literal(path) => {
            configuration.directories.insert(path);
        }
        DrizzleProperty::Missing => {
            if let Some(directory) = missing_directory {
                configuration.directories.insert(directory.to_string());
            }
        }
        DrizzleProperty::Dynamic => {}
    }
}

fn collect_drizzle_calls(
    node: Node<'_>,
    source: &str,
    define_config: &BTreeSet<String>,
    migrate: &BTreeSet<String>,
    configuration: &mut DrizzleConfiguration,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && let Ok(name) = function.utf8_text(source.as_bytes())
    {
        if define_config.contains(name) {
            record_drizzle_call(node, source, "out", 0, Some("drizzle"), configuration);
        } else if migrate.contains(name) {
            record_drizzle_call(node, source, "migrationsFolder", 1, None, configuration);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_drizzle_calls(child, source, define_config, migrate, configuration);
    }
}

fn drizzle_tree(source_path: &str, source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    let language = if Path::new(source_path)
        .extension()
        .is_some_and(|extension| extension == "tsx")
    {
        tree_sitter_typescript::LANGUAGE_TSX
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    };
    parser.set_language(&language.into()).ok()?;
    crate::typescript_compat::parse(&mut parser, source, source_path).ok()
}

fn inspect_drizzle_source(
    source_path: &str,
    source: &str,
    configuration: &mut DrizzleConfiguration,
) {
    let Some(imports) = actual_imports(source_path, source) else {
        return;
    };
    let define_config = typescript_imported_names(&imports, "drizzle-kit", "defineConfig");
    let migrate = typescript_imported_names(&imports, "drizzle-orm", "migrate");
    if define_config.is_empty() && migrate.is_empty() {
        return;
    }
    let Some(tree) = drizzle_tree(source_path, source) else {
        return;
    };
    collect_drizzle_calls(
        tree.root_node(),
        source,
        &define_config,
        &migrate,
        configuration,
    );
}

fn resolved_drizzle_directories(configuration: DrizzleConfiguration) -> Option<BTreeSet<String>> {
    (!configuration.directories.is_empty()).then_some(configuration.directories)
}

fn drizzle_directories(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> Option<BTreeSet<String>> {
    let mut configuration = DrizzleConfiguration::default();
    for source_path in &implementation.source_files {
        let Some(source) = files.get(source_path) else {
            continue;
        };
        inspect_drizzle_source(source_path, source, &mut configuration);
    }
    resolved_drizzle_directories(configuration)
}

fn drizzle_owns_path(
    index: &MigrationArtifactIndex,
    implementation: &Implementation,
    path: &str,
) -> bool {
    let target = implementation_relative_path(implementation, path);
    index
        .drizzle_directories
        .get(&implementation.id)
        .is_some_and(|directories| {
            directories
                .iter()
                .any(|directory| Path::new(target).starts_with(directory))
        })
}

pub fn migration_artifact_index(
    rule_pack: &str,
    files: &BTreeMap<String, String>,
    implementations: &[Implementation],
) -> MigrationArtifactIndex {
    let mut index = MigrationArtifactIndex::default();
    if rule_pack == "typescript-v1" {
        for implementation in implementations {
            if let Some(directories) = drizzle_directories(files, implementation) {
                index
                    .drizzle_directories
                    .insert(implementation.id.clone(), directories);
            }
        }
    }
    index
}

fn typescript_migration_artifact_for(
    index: &MigrationArtifactIndex,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    static VERSIONED_SQL: OnceLock<Regex> = OnceLock::new();
    static PRISMA_SQL: OnceLock<Regex> = OnceLock::new();
    if matches(
        path,
        &PRISMA_SQL,
        r"(?:^|/)prisma/migrations/[^/]+/migration\.sql$",
    ) {
        return Some(artifact(
            "prisma_7",
            &implementation.id,
            path,
            [
                "canonical_path:prisma_migrate",
                "canonical_filename:migration_sql",
            ],
        ));
    }
    if drizzle_owns_path(index, implementation, path)
        && matches(path, &VERSIONED_SQL, r"(?:^|/)[0-9]+_[^/]+\.sql$")
    {
        return Some(artifact(
            "drizzle_kit",
            &implementation.id,
            path,
            ["framework_source:drizzle", "versioned_sql_filename"],
        ));
    }
    None
}

fn rust_migration_artifact_for(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    static DIESEL_SQL: OnceLock<Regex> = OnceLock::new();
    static REFINERY_SQL: OnceLock<Regex> = OnceLock::new();
    static VERSIONED_SQL: OnceLock<Regex> = OnceLock::new();
    if diesel_embeds_path(files, implementation, path)
        && matches(path, &DIESEL_SQL, r"(?:^|/)[0-9][^/]*/(?:up|down)\.sql$")
    {
        return Some(artifact(
            "diesel",
            &implementation.id,
            path,
            [
                "embedding_source:diesel_migrations",
                "canonical_filename:up_or_down_sql",
            ],
        ));
    }
    if refinery_embeds_path(files, implementation, path)
        && matches(path, &REFINERY_SQL, r"(?:^|/)[UV][0-9]+__[^/]+\.sql$")
    {
        return Some(artifact(
            "refinery",
            &implementation.id,
            path,
            ["framework_source:refinery", "versioned_sql_filename"],
        ));
    }
    if sqlx_embeds_path(files, implementation, path)
        && matches(
            path,
            &VERSIONED_SQL,
            r"(?:^|/)[0-9]+_[^/]+(?:\.(?:up|down))?\.sql$",
        )
    {
        return Some(artifact(
            "sqlx",
            &implementation.id,
            path,
            ["embedding_source:sqlx_migrate", "versioned_sql_filename"],
        ));
    }
    None
}

fn migration_artifact_for(
    rule_pack: &str,
    files: &BTreeMap<String, String>,
    index: &MigrationArtifactIndex,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    match rule_pack {
        "typescript-v1" => typescript_migration_artifact_for(index, implementation, path),
        "rust-v1" => rust_migration_artifact_for(files, implementation, path),
        _ => None,
    }
}

pub fn migration_artifact_path_candidate(rule_pack: &str, path: &str) -> bool {
    static VERSIONED_SQL: OnceLock<Regex> = OnceLock::new();
    static PRISMA_SQL: OnceLock<Regex> = OnceLock::new();
    static DIESEL_SQL: OnceLock<Regex> = OnceLock::new();
    static REFINERY_SQL: OnceLock<Regex> = OnceLock::new();
    match rule_pack {
        "typescript-v1" => {
            matches(path, &VERSIONED_SQL, r"(?:^|/)[0-9]+_[^/]+\.sql$")
                || matches(
                    path,
                    &PRISMA_SQL,
                    r"(?:^|/)prisma/migrations/[^/]+/migration\.sql$",
                )
        }
        "rust-v1" => {
            matches(path, &DIESEL_SQL, r"(?:^|/)[0-9][^/]*/(?:up|down)\.sql$")
                || matches(
                    path,
                    &VERSIONED_SQL,
                    r"(?:^|/)[0-9]+_[^/]+(?:\.(?:up|down))?\.sql$",
                )
                || matches(path, &REFINERY_SQL, r"(?:^|/)[UV][0-9]+__[^/]+\.sql$")
        }
        _ => false,
    }
}

pub fn is_recognized_migration_artifact(
    rule_pack: &str,
    files: &BTreeMap<String, String>,
    index: &MigrationArtifactIndex,
    implementations: &[Implementation],
    path: &str,
) -> bool {
    implementation_for_artifact(implementations, path).is_some_and(|implementation| {
        migration_artifact_for(rule_pack, files, index, implementation, path).is_some()
    })
}

pub fn resolve_migration_artifacts(input: &Input) -> Vec<MigrationArtifact> {
    let index = migration_artifact_index(
        &input.policy.rule_pack,
        &input.files,
        &input.implementations,
    );
    input
        .artifacts
        .keys()
        .filter_map(|path| {
            let implementation = implementation_for_artifact(&input.implementations, path)?;
            migration_artifact_for(
                &input.policy.rule_pack,
                &input.files,
                &index,
                implementation,
                path,
            )
        })
        .collect()
}

pub fn resolve_source_role_candidates(input: &Input) -> Vec<SourceRoleCandidate> {
    static REFINERY_SOURCE_FILE: OnceLock<Regex> = OnceLock::new();
    let resolved = input
        .source_roles
        .iter()
        .map(|role| role.path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    input
        .files
        .iter()
        .filter(|(path, _)| !resolved.contains(path.as_str()))
        .filter_map(|(path, source)| {
            let mut evidence = Vec::new();
            if let Some(component) = path
                .split('/')
                .find(|component| matches!(*component, "migration" | "migrations" | "versions"))
            {
                evidence.push(format!("conventional_path_component:{component}"));
            }
            if input.policy.rule_pack == "rust-v1"
                && matches(
                    path,
                    &REFINERY_SOURCE_FILE,
                    r"(?:^|/)[UV][0-9]+__[^/]+\.rs$",
                )
            {
                evidence.push("unresolved_versioned_filename:refinery".into());
            }
            let package_signals: &[(&str, &str)] = match input.policy.rule_pack.as_str() {
                "python-v1" => &[("alembic", "alembic"), ("django.db", "django_migrations")],
                "typescript-v1" => &[
                    ("typeorm", "typeorm"),
                    ("@mikro-orm/migrations", "mikro_orm_migrations"),
                    ("kysely", "kysely"),
                    ("knex", "knex"),
                    ("umzug", "umzug"),
                    ("sequelize", "sequelize"),
                    ("@prisma/migrations", "prisma_migrations"),
                ],
                "rust-v1" => &[
                    ("sea_orm_migration", "sea_orm_migration"),
                    ("refinery", "refinery"),
                    ("rusqlite_migration", "rusqlite_migration"),
                ],
                _ => &[],
            };
            for (needle, label) in package_signals {
                if source.contains(needle) {
                    evidence.push(format!("unresolved_framework_signal:{label}"));
                }
            }
            if evidence.is_empty() {
                return None;
            }
            Some(SourceRoleCandidate {
                role: "possible_database_migration".into(),
                path: path.clone(),
                confidence: "low".into(),
                evidence,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_glob_matches_zero_or_more_directories() {
        let matcher = glob_regex("/backend/alembic/versions/**/*.py").unwrap();
        assert!(matcher.is_match("backend/alembic/versions/one.py"));
        assert!(matcher.is_match("backend/alembic/versions/tenant/one.py"));
        assert!(!matcher.is_match("other/alembic/versions/one.py"));
    }

    #[test]
    fn declared_typeorm_shell_accepts_an_aliased_interface() {
        let source = concat!(
            "import type { MigrationInterface as AppMigration, QueryRunner } from 'typeorm';\n",
            "export class Accounts implements AppMigration {\n",
            "  async up(queryRunner: QueryRunner) {}\n",
            "  async down(queryRunner: QueryRunner) {}\n",
            "}\n",
        );
        assert!(class_shell_matches("typeorm", source, "Accounts", true));
    }
}
