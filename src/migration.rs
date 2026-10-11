use crate::{
    core::migration::{
        CanonicalDeclaration, CodeMigrationFramework, TypeScriptClassContract,
        TypeScriptExportContract, code_migration_framework, leaf_symbol, reported_role,
    },
    core::model::{
        Implementation, Input, MigrationArtifact, MigrationArtifactCandidate, MigrationIntegrity,
        ResolvedSourceRole, RoleContext, SourceRoleCandidate, SourceSpan,
    },
    policy::{Policy, SourceRoleDeclaration},
    report::Location,
};
use regex::Regex;
use std::sync::OnceLock;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use tree_sitter::{Node, Parser};

#[derive(Default)]
pub struct MigrationArtifactIndex {
    drizzle_directories: BTreeMap<String, BTreeSet<String>>,
    prisma_7_directories: BTreeMap<String, BTreeSet<String>>,
    prisma_8_directories: BTreeMap<String, BTreeSet<String>>,
    diesel_directories: BTreeMap<String, BTreeSet<String>>,
    refinery_directories: BTreeMap<String, BTreeSet<String>>,
    sqlx_directories: BTreeMap<String, BTreeSet<String>>,
    directory_declarations: BTreeMap<(String, String, String), ArtifactDeclaration>,
}

#[derive(Clone, Debug)]
struct ArtifactDeclaration {
    path: String,
    span: SourceSpan,
}

fn glob_regex(scope: &str) -> Result<Regex, String> {
    let relative = scope
        .strip_prefix('/')
        .ok_or_else(|| format!("source role path is not repository-root anchored: {scope}"))?;
    let mut expression = String::from("^");
    let mut characters = relative.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '*' {
            if characters.next_if_eq(&'*').is_some() {
                if characters.next_if_eq(&'/').is_some() {
                    expression.push_str("(?:.*/)?");
                } else {
                    expression.push_str(".*");
                }
            } else {
                expression.push_str("[^/]*");
            }
        } else {
            expression.push_str(&regex::escape(&character.to_string()));
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
            .map(|path| format!("policy_configuration_path_declared_unverified:{path}")),
    );
    ResolvedSourceRole {
        role: declaration.role.clone(),
        framework: declaration.framework.clone(),
        implementation_id: implementation_id.into(),
        path: path.into(),
        path_scope: path_scope.into(),
        source: "declared".into(),
        confidence: "explicit".into(),
        contract_version: migration_contract_version(&declaration.framework).into(),
        configuration_status: (!declaration.configuration.is_empty())
            .then(|| "declared_unverified".into()),
        declaration_span: None,
        evidence,
    }
}

pub fn resolve_source_roles(
    policy: &Policy,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
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
    resolve_inferred_source_roles(language, files, manifests, implementations, &mut resolved)?;
    for role in resolved.values_mut() {
        role.declaration_span = files.get(&role.path).and_then(|source| {
            migration_declaration_span(
                &role.framework,
                &role.path,
                source,
                role.source == "declared",
            )
        });
    }
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
    manifests: &BTreeMap<String, String>,
    implementations: &[Implementation],
    resolved: &mut BTreeMap<String, ResolvedSourceRole>,
) -> Result<(), String> {
    for (path, source) in files {
        if resolved.contains_key(path) {
            continue;
        }
        let implementation = implementation_for_source(implementations, path)
            .ok_or_else(|| format!("source file has no Implementation owner: {path}"))?;
        if let Some(role) =
            inferred_source_role(language, path, source, files, manifests, implementation)
        {
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

fn migration_contract_version(framework: &str) -> &'static str {
    const CONTRACTS: &[(&str, &str)] = &[
        ("alembic", "alembic:revision-v1"),
        ("django", "django:migration-class-v1"),
        ("typeorm", "typeorm:migration-interface-v1"),
        ("knex", "knex:named-exports-v1"),
        ("mikro_orm", "mikro-orm:migration-class-v1"),
        ("kysely", "kysely:named-exports-v1"),
        ("sequelize_umzug", "umzug:typescript-v3"),
        ("prisma_8", "prisma:8-rc"),
        ("sea_orm", "sea-orm:migration-trait-v1"),
        ("refinery", "refinery:versioned-rust-v1"),
        ("rusqlite_migration", "rusqlite-migration:sequence-v1"),
    ];
    CONTRACTS
        .iter()
        .find_map(|(name, contract)| (*name == framework).then_some(*contract))
        .unwrap_or("migration:unknown")
}

fn source_position(source: &str, byte: usize) -> (usize, usize) {
    let prefix = &source[..byte.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, suffix)| suffix)
        .chars()
        .count()
        + 1;
    (line, column)
}

fn source_span(source: &str, start: usize, end: usize) -> SourceSpan {
    let (start_line, start_column) = source_position(source, start);
    let (end_line, end_column) = source_position(source, end);
    SourceSpan {
        start_line,
        start_column,
        end_line,
        end_column,
    }
}

fn migration_declaration_span(
    framework: &str,
    path: &str,
    source: &str,
    declared: bool,
) -> Option<SourceSpan> {
    let code = code_only(path, source)?;
    let pattern = migration_declaration_pattern(framework, path, source, declared)?;
    Regex::new(&pattern)
        .expect("static migration declaration span regex")
        .find(&code)
        .map(|matched| source_span(source, matched.start(), matched.end()))
}

fn migration_declaration_pattern(
    framework: &str,
    path: &str,
    source: &str,
    declared: bool,
) -> Option<String> {
    static_migration_declaration_pattern(framework)
        .map(str::to_string)
        .or_else(|| {
            let contract = typescript_class_contract(framework)?;
            typescript_class_declaration_pattern(path, source, declared, contract)
        })
}

fn static_migration_declaration_pattern(framework: &str) -> Option<&'static str> {
    const PATTERNS: &[(&str, &str)] = &[
        (
            "alembic",
            r"(?m)^(?:async[ \t]+)?def[ \t]+(?:upgrade|upgrade_[A-Za-z0-9_]+)[ \t]*\([^)]*\)[^:\n]*:",
        ),
        (
            "django",
            r"(?m)^[ \t]*class[ \t]+Migration[ \t]*\([^\n]*\)[ \t]*:",
        ),
        (
            "knex",
            r"(?m)^[ \t]*export[ \t]+(?:async[ \t]+)?(?:function[ \t]+up\b|const[ \t]+up\b)[^\n]*",
        ),
        (
            "kysely",
            r"(?m)^[ \t]*export[ \t]+(?:async[ \t]+)?(?:function[ \t]+up\b|const[ \t]+up\b)[^\n]*",
        ),
        (
            "sequelize_umzug",
            r"(?m)^[ \t]*export[ \t]+(?:async[ \t]+)?(?:function[ \t]+up\b|const[ \t]+up\b)[^\n]*",
        ),
        (
            "sea_orm",
            r"(?m)^[ \t]*impl[ \t]+MigrationTrait[ \t]+for[ \t]+Migration[^\n{]*\{",
        ),
        (
            "refinery",
            r"(?m)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?fn[ \t]+migration[ \t]*\([^)]*\)[^\n{]*",
        ),
    ];
    PATTERNS
        .iter()
        .find_map(|(name, pattern)| (*name == framework).then_some(*pattern))
}

fn typescript_class_contract(framework: &str) -> Option<TypeScriptClassContract> {
    match framework {
        "typeorm" => Some(TypeScriptClassContract::TypeOrm),
        "mikro_orm" => Some(TypeScriptClassContract::MikroOrm),
        "prisma_8" => Some(TypeScriptClassContract::Prisma8),
        _ => None,
    }
}

fn typescript_class_declaration_pattern(
    path: &str,
    source: &str,
    declared: bool,
    contract: TypeScriptClassContract,
) -> Option<String> {
    let imports = actual_imports(path, source)?;
    let class = typescript_class_shapes(source)?
        .into_iter()
        .find(|class| typescript_class_matches_contract(class, &imports, declared, contract))?;
    Some(if class.name == "<anonymous>" {
        String::from(r"(?m)^[ \t]*export[ \t]+default[ \t]+class[^\n{]*\{")
    } else {
        format!(
            r"(?m)^[ \t]*(?:export[ \t]+(?:default[ \t]+)?)?(?:abstract[ \t]+)?class[ \t]+{}\b[^\n{{]*\{{",
            regex::escape(&class.name)
        )
    })
}

fn typescript_class_matches_contract(
    class: &TypescriptClassShape,
    imports: &[String],
    declared: bool,
    contract: TypeScriptClassContract,
) -> bool {
    match contract {
        TypeScriptClassContract::TypeOrm => typeorm_class_shell(class, imports, declared),
        TypeScriptClassContract::MikroOrm => mikro_orm_class_shell(class, imports, declared),
        TypeScriptClassContract::Prisma8 => prisma_class_shell(class, imports, declared),
    }
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

fn alembic_multidatabase_entrypoints(source: &str) -> bool {
    let dispatcher = |name: &str| {
        Regex::new(&format!(
            r"(?m)^(?:async[ \t]+)?def[ \t]+{}[ \t]*\([ \t]*[A-Za-z_][A-Za-z0-9_]*(?:[ \t]*:[^)]*)?\)[ \t]*(?:->[ \t]*[^:\n]+)?[ \t]*:",
            regex::escape(name)
        ))
        .expect("escaped Alembic dispatcher name")
        .is_match(source)
    };
    if !(dispatcher("upgrade") && dispatcher("downgrade")) {
        return false;
    }
    let suffixes = |direction: &str| {
        Regex::new(&format!(
            r"(?m)^(?:async[ \t]+)?def[ \t]+{}_(\w+)[ \t]*\([ \t]*\)[ \t]*(?:->[ \t]*[^:\n]+)?[ \t]*:",
            regex::escape(direction)
        ))
        .expect("escaped Alembic multidatabase direction")
        .captures_iter(source)
        .map(|capture| capture[1].to_string())
        .collect::<BTreeSet<_>>()
    };
    let upgrades = suffixes("upgrade");
    !upgrades.is_empty() && upgrades == suffixes("downgrade")
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
    let multidatabase = alembic_multidatabase_entrypoints(&code);
    if !(imported && alembic_revision_graph(&code) && (upgrade || multidatabase)) {
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
        contract_version: migration_contract_version("alembic").into(),
        configuration_status: None,
        declaration_span: None,
        evidence: vec![
            "framework_import:alembic".into(),
            "revision_graph_metadata".into(),
            if multidatabase {
                "canonical_entrypoints:multidatabase_dispatch".into()
            } else {
                "canonical_entrypoint:upgrade".into()
            },
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
    if imported && migration_class && django_migration_members(source) {
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

fn django_migration_members(source: &str) -> bool {
    let Some(tree) = parsed_python_tree(source) else {
        return false;
    };
    let mut root_cursor = tree.root_node().walk();
    tree.root_node()
        .named_children(&mut root_cursor)
        .find(|class| django_migration_class(*class, source))
        .and_then(|class| class.child_by_field_name("body"))
        .is_some_and(|body| django_body_has_required_members(body, source))
}

fn parsed_python_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    (!tree.root_node().has_error()).then_some(tree)
}

fn django_migration_class(class: Node<'_>, source: &str) -> bool {
    class.kind() == "class_definition"
        && class
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source.as_bytes()).ok())
            == Some("Migration")
}

fn django_body_has_required_members(body: Node<'_>, source: &str) -> bool {
    let mut body_cursor = body.walk();
    let members = body
        .named_children(&mut body_cursor)
        .filter_map(django_assignment)
        .filter_map(|assignment| assignment.child_by_field_name("left"))
        .filter_map(|left| left.utf8_text(source.as_bytes()).ok())
        .filter(|name| matches!(*name, "dependencies" | "operations"))
        .collect::<BTreeSet<_>>();
    members.contains("dependencies") && members.contains("operations")
}

fn django_assignment(statement: Node<'_>) -> Option<Node<'_>> {
    let assignment = if statement.kind() == "expression_statement" {
        statement.named_child(0)
    } else {
        Some(statement)
    }?;
    (assignment.kind() == "assignment").then_some(assignment)
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
        contract_version: migration_contract_version(framework).into(),
        configuration_status: None,
        declaration_span: None,
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
    span: SourceSpan,
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
            span: source_span(source, node.start_byte(), node.end_byte()),
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
    contract: TypeScriptExportContract,
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
    package_bound_parameter(contract, &parameters, imports)
        || (contract == TypeScriptExportContract::SequelizeUmzug
            && umzug_migration_annotation(code, imports, name))
}

fn package_bound_parameter(
    contract: TypeScriptExportContract,
    parameters: &str,
    imports: &[String],
) -> bool {
    match contract {
        TypeScriptExportContract::Knex => {
            parameter_uses_import(parameters, imports, "knex", "Knex")
        }
        TypeScriptExportContract::Kysely => {
            parameter_uses_import(parameters, imports, "kysely", "Kysely")
        }
        TypeScriptExportContract::SequelizeUmzug => {
            parameter_uses_import(parameters, imports, "sequelize", "QueryInterface")
        }
    }
}

fn umzug_migration_annotation(code: &str, imports: &[String], name: &str) -> bool {
    let Some(annotation) = exported_typescript_annotation(code, name) else {
        return false;
    };
    let mut migration_types = typescript_imported_names(imports, "umzug", "Migration");
    let local_module = Regex::new(r#"\bfrom\s*['\"](\.[^'\"]*)['\"]"#)
        .expect("static local TypeScript import regex");
    for statement in imports {
        if let Some(module) = local_module.captures(statement) {
            migration_types.extend(typescript_imported_names(
                std::slice::from_ref(statement),
                &module[1],
                "Migration",
            ));
        }
    }
    migration_types.iter().any(|imported| {
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
        && classes.iter().any(|class| {
            class_shell_matches(TypeScriptClassContract::TypeOrm, source, &class.name, false)
        })
}

fn inferred_mikro_orm(imports: &[String], classes: &[TypescriptClassShape], source: &str) -> bool {
    imported_from(imports, "@mikro-orm/migrations")
        && classes.iter().any(|class| {
            class_shell_matches(
                TypeScriptClassContract::MikroOrm,
                source,
                &class.name,
                false,
            )
        })
}

fn inferred_kysely(imports: &[String], code: &str) -> bool {
    imported_from(imports, "kysely")
        && exported_typescript_function(code, "up")
        && typescript_export_parameter_matches(
            TypeScriptExportContract::Kysely,
            code,
            imports,
            "up",
            false,
        )
}

fn inferred_knex(imports: &[String], code: &str) -> bool {
    imported_from(imports, "knex")
        && exported_typescript_function(code, "up")
        && exported_typescript_function(code, "down")
        && typescript_export_parameter_matches(
            TypeScriptExportContract::Knex,
            code,
            imports,
            "up",
            false,
        )
        && typescript_export_parameter_matches(
            TypeScriptExportContract::Knex,
            code,
            imports,
            "down",
            false,
        )
}

fn sequelize_entrypoints_use_query_interface(imports: &[String], code: &str) -> bool {
    ["up", "down"].iter().all(|name| {
        exported_typescript_parameters(code, name).is_some_and(|parameters| {
            package_bound_parameter(
                TypeScriptExportContract::SequelizeUmzug,
                &parameters,
                imports,
            )
        })
    })
}

fn typed_umzug_has_database_context(
    imports: &[String],
    code: &str,
    local_database_context: bool,
) -> bool {
    if !umzug_migration_annotation(code, imports, "up") {
        return false;
    }
    if exported_typescript_function(code, "down")
        && !umzug_migration_annotation(code, imports, "down")
    {
        return false;
    }
    let entrypoints = ["up", "down"]
        .into_iter()
        .filter(|name| exported_typescript_function(code, name));
    let typed_query_interface = entrypoints.clone().all(|name| {
        exported_typescript_annotation(code, name).is_some_and(|annotation| {
            parameter_uses_import(&annotation, imports, "sequelize", "QueryInterface")
        })
    });
    local_database_context
        || typed_query_interface
        || code_constructs_import(code, imports, "umzug", "SequelizeStorage")
}

fn inferred_sequelize_umzug(
    imports: &[String],
    code: &str,
    local_database_context: bool,
) -> Option<&'static str> {
    let sequelize_context = imported_from(imports, "sequelize")
        && !typescript_imported_names(imports, "sequelize", "QueryInterface").is_empty();
    if !(imported_from(imports, "umzug") || sequelize_context || local_database_context) {
        return None;
    }
    if !(sequelize_entrypoints_use_query_interface(imports, code)
        || typed_umzug_has_database_context(imports, code, local_database_context))
    {
        return None;
    }
    Some(if local_database_context {
        "framework_import:local_umzug_migration_alias"
    } else if sequelize_context {
        "framework_import:sequelize"
    } else {
        "framework_import:umzug"
    })
}

fn inferred_typescript(
    path: &str,
    source: &str,
    implementation_id: &str,
    database_umzug_alias_modules: &BTreeSet<String>,
    prisma_8_rc_dependency: bool,
) -> Option<ResolvedSourceRole> {
    let code = code_only(path, source)?;
    let imports = actual_imports(path, source)?;
    let classes = typescript_class_shapes(source)?;
    let local_umzug_database_context =
        imports_database_umzug_alias(path, &imports, database_umzug_alias_modules);
    inferred_typescript_class_role(&imports, &classes, source, path, implementation_id)
        .or_else(|| inferred_typescript_export_role(&imports, &code, path, implementation_id))
        .or_else(|| {
            inferred_typescript_umzug_role(
                &imports,
                &code,
                local_umzug_database_context,
                path,
                implementation_id,
            )
        })
        .or_else(|| {
            inferred_prisma_8_role(
                &imports,
                &classes,
                prisma_8_rc_dependency,
                path,
                implementation_id,
            )
        })
}

fn inferred_typescript_class_role(
    imports: &[String],
    classes: &[TypescriptClassShape],
    source: &str,
    path: &str,
    implementation_id: &str,
) -> Option<ResolvedSourceRole> {
    if inferred_typeorm(imports, classes, source) {
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
    if inferred_mikro_orm(imports, classes, source) {
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
    None
}

fn inferred_typescript_export_role(
    imports: &[String],
    code: &str,
    path: &str,
    implementation_id: &str,
) -> Option<ResolvedSourceRole> {
    if inferred_kysely(imports, code) {
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
    if inferred_knex(imports, code) {
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
    None
}

fn inferred_typescript_umzug_role(
    imports: &[String],
    code: &str,
    local_umzug_database_context: bool,
    path: &str,
    implementation_id: &str,
) -> Option<ResolvedSourceRole> {
    if let Some(framework_import) =
        inferred_sequelize_umzug(imports, code, local_umzug_database_context)
    {
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

fn inferred_prisma_8_role(
    imports: &[String],
    classes: &[TypescriptClassShape],
    prisma_8_rc_dependency: bool,
    path: &str,
    implementation_id: &str,
) -> Option<ResolvedSourceRole> {
    (prisma_8_rc_dependency
        && classes
            .iter()
            .any(|class| prisma_class_shell(class, imports, false)))
    .then(|| {
        inferred_role(
            "prisma_8",
            path,
            implementation_id,
            [
                "framework_import:prisma_8_migration",
                "package_version:prisma_8_rc",
                "migration_contract:extends_Migration",
                "canonical_getter:operations",
            ],
        )
    })
}

fn implementation_database_umzug_alias_modules(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> BTreeSet<String> {
    let exported_alias = Regex::new(
        r"(?m)^\s*export\s+type\s+Migration\s*=\s*typeof\s+[A-Za-z_$][A-Za-z0-9_$]*\._types\.migration\s*;?",
    )
    .expect("static Umzug exported migration alias regex");
    implementation
        .source_files
        .iter()
        .filter_map(|path| {
            files.get(path).and_then(|source| {
                let code = code_only(path, source)?;
                let imports = actual_imports(path, source)?;
                (imported_from(&imports, "umzug")
                    && code_constructs_import(&code, &imports, "umzug", "Umzug")
                    && code_constructs_import(&code, &imports, "umzug", "SequelizeStorage")
                    && exported_alias.is_match(&code))
                .then(|| typescript_module_key(path))
            })
        })
        .collect()
}

fn typescript_module_key(path: &str) -> String {
    [".tsx", ".mts", ".cts", ".ts"]
        .into_iter()
        .find_map(|extension| path.strip_suffix(extension))
        .unwrap_or(path)
        .to_string()
}

fn resolve_relative_typescript_module(importer: &str, module: &str) -> Option<String> {
    if !module.starts_with('.') || module.contains('\\') {
        return None;
    }
    let mut components = importer.rsplit_once('/').map_or(Vec::new(), |(parent, _)| {
        parent.split('/').map(str::to_string).collect()
    });
    for component in module.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component => components.push(component.to_string()),
        }
    }
    Some(typescript_module_key(&components.join("/")))
}

fn imports_database_umzug_alias(
    path: &str,
    imports: &[String],
    database_alias_modules: &BTreeSet<String>,
) -> bool {
    let local_module = Regex::new(r#"\bfrom\s*['\"](\.[^'\"]*)['\"]"#)
        .expect("static local TypeScript import regex");
    imports.iter().any(|statement| {
        let Some(capture) = local_module.captures(statement) else {
            return false;
        };
        let module = &capture[1];
        if typescript_imported_names(std::slice::from_ref(statement), module, "Migration")
            .is_empty()
        {
            return false;
        }
        resolve_relative_typescript_module(path, module).is_some_and(|resolved| {
            database_alias_modules.contains(&resolved)
                || database_alias_modules.contains(&format!("{resolved}/index"))
        })
    })
}

fn prisma_8_rc_version(requirement: &str) -> bool {
    requirement.split("||").any(|alternative| {
        alternative
            .trim()
            .trim_start_matches(['=', '^', '~', '>', '<', ' '])
            .starts_with("8.0.0-rc.")
    })
}

fn implementation_has_prisma_8_rc(
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> bool {
    implementation.runtime_manifests.iter().any(|path| {
        if !path.ends_with("package.json") {
            return false;
        }
        let Some(manifest) = manifests
            .get(path)
            .and_then(|source| serde_json::from_str::<serde_json::Value>(source).ok())
        else {
            return false;
        };
        [
            "dependencies",
            "devDependencies",
            "optionalDependencies",
            "peerDependencies",
        ]
        .into_iter()
        .filter_map(|section| manifest.get(section).and_then(serde_json::Value::as_object))
        .any(|dependencies| {
            ["@prisma/orm-postgres", "@prisma/orm-mongo"]
                .into_iter()
                .filter_map(|package| {
                    dependencies
                        .get(package)
                        .and_then(serde_json::Value::as_str)
                })
                .any(prisma_8_rc_version)
        })
    })
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
            let default_match = default
                .find_iter(source)
                .any(|matched| code[matched.range()].contains("embed_migrations!"));
            explicit_match || (default_match && Path::new(target).starts_with("migrations"))
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
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> Option<ResolvedSourceRole> {
    match language {
        "python" => inferred_alembic(path, source, &implementation.id)
            .or_else(|| inferred_django(path, source, &implementation.id)),
        "typescript" => inferred_typescript(
            path,
            source,
            &implementation.id,
            &implementation_database_umzug_alias_modules(files, implementation),
            implementation_has_prisma_8_rc(manifests, implementation),
        ),
        "rust" => inferred_rust(path, source, files, implementation),
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
    (class_has_framework_relation(
        class,
        imports,
        "@prisma/orm-postgres",
        "Migration",
        "extends",
        allow_declared_alias,
    ) || class_has_framework_relation(
        class,
        imports,
        "@prisma/orm-mongo",
        "Migration",
        "extends",
        allow_declared_alias,
    )) && class.methods.get("operations").is_some_and(|method| {
        Regex::new(r"\bget\s+operations\s*\(")
            .expect("static Prisma operations getter regex")
            .is_match(method)
    })
}

fn class_shell_matches(
    contract: TypeScriptClassContract,
    source: &str,
    symbol: &str,
    allow_declared_alias: bool,
) -> bool {
    let imports = actual_imports("migration.ts", source).unwrap_or_default();
    let classes = typescript_class_shapes(source).unwrap_or_default();
    let matches = |class: &TypescriptClassShape| match contract {
        TypeScriptClassContract::TypeOrm => {
            typeorm_class_shell(class, &imports, allow_declared_alias)
        }
        TypeScriptClassContract::MikroOrm => {
            mikro_orm_class_shell(class, &imports, allow_declared_alias)
        }
        TypeScriptClassContract::Prisma8 => {
            prisma_class_shell(class, &imports, allow_declared_alias)
        }
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
    let zero_argument_entrypoint = Regex::new(&format!(
        r"(?m)^(?:async[ \t]+)?def[ \t]+{}[ \t]*\([ \t]*\)[ \t]*(?:->[ \t]*[^:\n]+)?[ \t]*:",
        regex::escape(name)
    ))
    .expect("escaped Python function name")
    .is_match(&code);
    let multidatabase_entrypoint = name
        .strip_prefix("upgrade_")
        .or_else(|| name.strip_prefix("downgrade_"))
        .is_some_and(|suffix| !suffix.is_empty())
        && alembic_multidatabase_entrypoints(&code);
    owner.is_none()
        && alembic_revision_graph(&code)
        && zero_argument_entrypoint
        && (matches!(name, "upgrade" | "downgrade") || multidatabase_entrypoint)
}

fn canonical_typescript_class(
    contract: TypeScriptClassContract,
    source: &str,
    owner: Option<&str>,
    declared: bool,
) -> bool {
    owner.is_some_and(|owner| class_shell_matches(contract, source, leaf_symbol(owner), declared))
}

fn canonical_typescript_export(
    contract: TypeScriptExportContract,
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
        && typescript_export_parameter_matches(contract, &code, &imports, name, declared)
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

fn canonical_sea_orm(source: &str, name: &str, owner: Option<&str>) -> bool {
    let Some(code) = code_only("migration.rs", source) else {
        return false;
    };
    matches!(name, "up" | "down")
        && owner.is_some_and(|owner| leaf_symbol(owner) == "Migration")
        && code.contains("impl MigrationTrait for Migration")
        && unique_sea_orm_migration_type(&code)
}

fn canonical_function(
    framework: &CodeMigrationFramework,
    source: &str,
    symbol: &str,
    owner: Option<&str>,
    declared: bool,
) -> bool {
    let name = leaf_symbol(symbol);
    match framework.canonical_declaration() {
        CanonicalDeclaration::AlembicFunction => canonical_alembic(source, name, owner),
        CanonicalDeclaration::TypeScriptClass(contract) => {
            canonical_typescript_class(contract, source, owner, declared)
        }
        CanonicalDeclaration::TypeScriptExport(contract) => {
            canonical_typescript_export(contract, source, name, owner, declared)
        }
        CanonicalDeclaration::RefineryFunction => canonical_refinery(source, name, owner),
        CanonicalDeclaration::SeaOrmFunction => canonical_sea_orm(source, name, owner),
    }
}

fn exact_declaration_span(source: &str, location: &Location) -> Option<SourceSpan> {
    let line = source.lines().nth(location.line.checked_sub(1)?)?;
    let end_column = line.chars().count() + 1;
    (location.column <= end_column).then_some(SourceSpan {
        start_line: location.line,
        start_column: location.column,
        end_line: location.line,
        end_column,
    })
}

fn reported_role_at(
    source_role: &ResolvedSourceRole,
    declaration_role: &str,
    canonical: bool,
    source: &str,
    location: &Location,
) -> Option<RoleContext> {
    let mut context = reported_role(source_role, declaration_role, canonical)?;
    context.declaration_span = exact_declaration_span(source, location);
    Some(context)
}

pub fn function_role(
    input: &Input,
    path: &str,
    symbol: &str,
    owner: Option<&str>,
    location: &Location,
) -> Option<RoleContext> {
    let role = input.source_roles.iter().find(|role| role.path == path)?;
    let source = input.files.get(path)?;
    let framework = code_migration_framework(&role.framework)?;
    let declaration_role = framework.declaration_role(symbol)?;
    let canonical = canonical_function(framework, source, symbol, owner, role.source == "declared");
    reported_role_at(role, declaration_role, canonical, source, location)
}

pub fn class_role(
    input: &Input,
    path: &str,
    symbol: &str,
    location: &Location,
) -> Option<RoleContext> {
    let source_role = input.source_roles.iter().find(|role| role.path == path)?;
    let source = input.files.get(path)?;
    let CanonicalDeclaration::TypeScriptClass(contract) =
        code_migration_framework(&source_role.framework)?.canonical_declaration()
    else {
        return None;
    };
    let canonical = class_shell_matches(contract, source, symbol, source_role.source == "declared");
    reported_role_at(source_role, "migration_shell", canonical, source, location)
}

pub fn type_role(
    input: &Input,
    path: &str,
    symbol: &str,
    location: &Location,
) -> Option<RoleContext> {
    let source_role = input.source_roles.iter().find(|role| role.path == path)?;
    if source_role.framework != "sea_orm" || leaf_symbol(symbol) != "Migration" {
        return None;
    }
    let source = input.files.get(path)?;
    let code = code_only(path, source)?;
    let canonical =
        code.contains("impl MigrationTrait for Migration") && unique_sea_orm_migration_type(&code);
    reported_role_at(source_role, "migration_shell", canonical, source, location)
}

fn artifact(
    framework: &str,
    contract_version: &str,
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
        contract_version: contract_version.into(),
        declaration_path: None,
        declaration_span: None,
        integrity: None,
        evidence: evidence.into_iter().map(str::to_string).collect(),
    }
}

fn bundle_artifact(
    role: &str,
    framework: &str,
    implementation_id: &str,
    path: &str,
    status: &str,
    contract_version: &str,
    evidence: impl IntoIterator<Item = &'static str>,
) -> MigrationArtifact {
    MigrationArtifact {
        role: role.into(),
        framework: framework.into(),
        implementation_id: implementation_id.into(),
        path: path.into(),
        status: status.into(),
        contract_version: contract_version.into(),
        declaration_path: None,
        declaration_span: None,
        integrity: None,
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
    declarations: BTreeMap<String, ArtifactDeclaration>,
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
    source_path: &str,
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
            configuration.directories.insert(path.clone());
            configuration.declarations.insert(
                path,
                ArtifactDeclaration {
                    path: source_path.to_string(),
                    span: source_span(source, node.start_byte(), node.end_byte()),
                },
            );
        }
        DrizzleProperty::Missing => {
            if let Some(directory) = missing_directory {
                configuration.directories.insert(directory.to_string());
                configuration.declarations.insert(
                    directory.to_string(),
                    ArtifactDeclaration {
                        path: source_path.to_string(),
                        span: source_span(source, node.start_byte(), node.end_byte()),
                    },
                );
            }
        }
        DrizzleProperty::Dynamic => {}
    }
}

fn collect_drizzle_calls(
    node: Node<'_>,
    source_path: &str,
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
            record_drizzle_call(
                node,
                source_path,
                source,
                "out",
                0,
                Some("drizzle"),
                configuration,
            );
        } else if migrate.contains(name) {
            record_drizzle_call(
                node,
                source_path,
                source,
                "migrationsFolder",
                1,
                None,
                configuration,
            );
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_drizzle_calls(
            child,
            source_path,
            source,
            define_config,
            migrate,
            configuration,
        );
    }
}

fn typescript_tree(source_path: &str, source: &str) -> Option<tree_sitter::Tree> {
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

fn collect_prisma_config_calls(
    node: Node<'_>,
    source_path: &str,
    source: &str,
    define_config: &BTreeSet<String>,
    directories: &mut BTreeSet<String>,
    declarations: &mut BTreeMap<String, ArtifactDeclaration>,
) {
    if let Some(path) = prisma_config_migrations_path(node, source, define_config) {
        directories.insert(path.clone());
        declarations.insert(
            path,
            ArtifactDeclaration {
                path: source_path.to_string(),
                span: source_span(source, node.start_byte(), node.end_byte()),
            },
        );
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_prisma_config_calls(
            child,
            source_path,
            source,
            define_config,
            directories,
            declarations,
        );
    }
}

fn prisma_config_migrations_path(
    node: Node<'_>,
    source: &str,
    define_config: &BTreeSet<String>,
) -> Option<String> {
    (node.kind() == "call_expression").then_some(())?;
    let function = node.child_by_field_name("function")?;
    let name = function.utf8_text(source.as_bytes()).ok()?;
    define_config.contains(name).then_some(())?;
    let arguments = node
        .child_by_field_name("arguments")
        .or_else(|| node.named_child(1))?;
    let mut argument_cursor = arguments.walk();
    let configuration = arguments.named_children(&mut argument_cursor).next()?;
    (configuration.kind() == "object").then_some(())?;
    let mut member_cursor = configuration.walk();
    configuration
        .named_children(&mut member_cursor)
        .find_map(|member| prisma_migrations_member(member, source))
}

fn prisma_migrations_member(member: Node<'_>, source: &str) -> Option<String> {
    (member.kind() == "pair").then_some(())?;
    let key = member
        .child_by_field_name("key")?
        .utf8_text(source.as_bytes())
        .ok()?
        .trim_matches(['\'', '"']);
    (key == "migrations").then_some(())?;
    let migrations = member.child_by_field_name("value")?;
    (migrations.kind() == "object").then_some(())?;
    match drizzle_property(migrations, source, "path") {
        DrizzleProperty::Literal(path) => Some(path),
        DrizzleProperty::Dynamic | DrizzleProperty::Missing => None,
    }
}

fn prisma_7_directories(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> (BTreeSet<String>, BTreeMap<String, ArtifactDeclaration>) {
    let mut directories = BTreeSet::new();
    let mut declarations = BTreeMap::new();
    for source_path in &implementation.source_files {
        let Some(source) = files.get(source_path) else {
            continue;
        };
        let Some(imports) = actual_imports(source_path, source) else {
            continue;
        };
        let define_config = typescript_imported_names(&imports, "prisma/config", "defineConfig");
        if define_config.is_empty() {
            continue;
        }
        let Some(tree) = typescript_tree(source_path, source) else {
            continue;
        };
        collect_prisma_config_calls(
            tree.root_node(),
            source_path,
            source,
            &define_config,
            &mut directories,
            &mut declarations,
        );
    }
    (directories, declarations)
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
    let Some(tree) = typescript_tree(source_path, source) else {
        return;
    };
    collect_drizzle_calls(
        tree.root_node(),
        source_path,
        source,
        &define_config,
        &migrate,
        configuration,
    );
}

fn resolved_drizzle_directories(
    configuration: DrizzleConfiguration,
) -> Option<DrizzleConfiguration> {
    (!configuration.directories.is_empty()).then_some(configuration)
}

fn drizzle_directories(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> Option<DrizzleConfiguration> {
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

fn cargo_declares_dependency(source: &str, dependency: &str) -> bool {
    cargo_dependency_span(source, dependency).is_some()
}

fn cargo_dependency_span(source: &str, dependency: &str) -> Option<SourceSpan> {
    let mut section = String::new();
    let mut offset = 0;
    for raw_line in source.split_inclusive('\n') {
        let line = toml_code(raw_line);
        let end = offset + raw_line.trim_end_matches(['\r', '\n']).len();
        if let Some(name) = toml_section(line) {
            section = name.to_string();
            if cargo_dependency_table(&section, dependency) {
                return Some(source_span(source, offset, end));
            }
        } else if cargo_dependency_section(&section) && toml_key(line) == Some(dependency) {
            return Some(source_span(source, offset, end));
        }
        offset += raw_line.len();
    }
    None
}

fn toml_code(raw_line: &str) -> &str {
    raw_line.split('#').next().unwrap_or_default().trim()
}

fn toml_section(line: &str) -> Option<&str> {
    line.strip_prefix('[')?.strip_suffix(']').map(str::trim)
}

fn toml_key(line: &str) -> Option<&str> {
    line.split_once('=')
        .map(|(key, _)| key.trim().trim_matches(['\'', '"']))
}

fn cargo_dependency_table(section: &str, dependency: &str) -> bool {
    section
        .rsplit_once('.')
        .is_some_and(|(prefix, name)| prefix.ends_with("dependencies") && name == dependency)
}

fn cargo_dependency_section(section: &str) -> bool {
    [
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "workspace.dependencies",
    ]
    .contains(&section)
        || [".dependencies", ".dev-dependencies", ".build-dependencies"]
            .iter()
            .any(|suffix| section.ends_with(suffix))
}

fn implementation_declares_dependency(
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
    dependency: &str,
) -> bool {
    implementation.runtime_manifests.iter().any(|path| {
        path.ends_with("Cargo.toml")
            && manifests
                .get(path)
                .is_some_and(|source| cargo_declares_dependency(source, dependency))
    })
}

fn literal_toml_setting(
    source: &str,
    section: &str,
    setting: &str,
) -> Option<(String, SourceSpan)> {
    let mut current_section = "";
    let mut offset = 0;
    for raw_line in source.split_inclusive('\n') {
        let line = toml_code(raw_line);
        if let Some(name) = toml_section(line) {
            current_section = name;
            offset += raw_line.len();
            continue;
        }
        if current_section != section {
            offset += raw_line.len();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            offset += raw_line.len();
            continue;
        };
        if key.trim() != setting {
            offset += raw_line.len();
            continue;
        }
        if let Some(path) = portable_toml_path(value) {
            let end = offset + raw_line.trim_end_matches(['\r', '\n']).len();
            return Some((path, source_span(source, offset, end)));
        }
        offset += raw_line.len();
    }
    None
}

fn portable_toml_path(value: &str) -> Option<String> {
    let value = value.trim();
    let path = value
        .strip_prefix(['\'', '"'])?
        .strip_suffix(['\'', '"'])?
        .trim_start_matches("./");
    (!path.is_empty()
        && !path.contains('\\')
        && !path.split('/').any(|component| component == ".."))
    .then(|| path.to_string())
}

fn cargo_dependency_declaration(
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
    dependency: &str,
) -> Option<ArtifactDeclaration> {
    implementation.runtime_manifests.iter().find_map(|path| {
        let source = path
            .ends_with("Cargo.toml")
            .then(|| manifests.get(path))??;
        cargo_dependency_span(source, dependency).map(|span| ArtifactDeclaration {
            path: path.clone(),
            span,
        })
    })
}

fn portable_migration_directory(path: &str) -> Option<String> {
    let path = path.trim().trim_start_matches("./");
    (!path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.split('/').any(|component| component == ".."))
    .then(|| path.to_string())
}

struct JustPatterns {
    recipe_header: Regex,
    command: Regex,
    path_argument: Regex,
}

impl JustPatterns {
    fn new() -> Self {
        Self {
            recipe_header: Regex::new(
                r"^@?[A-Za-z_][A-Za-z0-9_-]*(?:[ \t]+[^:=\n]+)*[ \t]*:(?:[ \t].*)?$",
            )
            .expect("static Just recipe header regex"),
            command: Regex::new(r"\brefinery(?:\.exe)?[ \t]+migrate\b")
                .expect("static Refinery CLI command regex"),
            path_argument: Regex::new(
                r#"(?:^|[ \t])(?:-p|--path)(?:[ \t]*=[ \t]*|[ \t]+)(?:\"([^\"]+)\"|'([^']+)'|([^ \t#;&|,'\"}]+))"#,
            )
            .expect("static Refinery CLI path regex"),
        }
    }
}

#[derive(Default)]
struct JustRecipeState {
    recipe_body: bool,
    script_recipe: bool,
    pending_script_attribute: bool,
    continued_recipe_line: bool,
    multiline_string: Option<&'static str>,
}

impl JustRecipeState {
    fn consumes_multiline_string(&mut self, raw_line: &str) -> bool {
        if let Some(delimiter) = self.multiline_string {
            if raw_line.matches(delimiter).count() % 2 == 1 {
                self.multiline_string = None;
            }
            return true;
        }
        let delimiter = ["'''", "\"\"\""]
            .into_iter()
            .find(|delimiter| raw_line.contains(delimiter));
        if let Some(delimiter) = delimiter {
            if raw_line.matches(delimiter).count() % 2 == 1 {
                self.multiline_string = Some(delimiter);
            }
            return true;
        }
        false
    }

    fn consumes_recipe_prefix(
        &mut self,
        authored_trimmed: &str,
        authored_indentation: usize,
        continues: bool,
    ) -> bool {
        if self.recipe_body && authored_indentation > 0 && self.continued_recipe_line {
            self.continued_recipe_line = continues;
            return true;
        }
        if self.recipe_body && authored_indentation > 0 && authored_trimmed.starts_with("#!") {
            self.script_recipe = true;
            return true;
        }
        false
    }

    fn consumes_top_level(&mut self, trimmed: &str, indentation: usize, header: &Regex) -> bool {
        if indentation != 0 {
            return false;
        }
        if trimmed.starts_with("[script") && trimmed.ends_with(']') {
            self.pending_script_attribute = true;
            self.recipe_body = false;
        } else if header.is_match(trimmed.trim_end()) {
            self.recipe_body = true;
            self.script_recipe = self.pending_script_attribute;
            self.pending_script_attribute = false;
            self.continued_recipe_line = false;
        } else if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
            self.recipe_body = false;
            self.script_recipe = false;
            self.pending_script_attribute = false;
            self.continued_recipe_line = false;
        }
        true
    }
}

fn refinery_cli_directories(source: &str) -> Vec<(String, SourceSpan)> {
    let working_directory_override = Regex::new(
        r"(?m)^[ \t]*(?:set[ \t]+(?:no-cd|working-directory)\b|\[(?:no-cd|working-directory)\b)",
    )
    .expect("static Just working-directory override regex");
    if working_directory_override.is_match(source) {
        return Vec::new();
    }
    let patterns = JustPatterns::new();
    let mut state = JustRecipeState::default();
    let mut result = Vec::new();
    let mut offset = 0;
    for raw_line in source.split_inclusive('\n') {
        if let Some(directory) =
            refinery_cli_directory_on_line(&mut state, &patterns, source, raw_line, offset)
        {
            result.push(directory);
        }
        offset += raw_line.len();
    }
    result
}

fn refinery_cli_directory_on_line(
    state: &mut JustRecipeState,
    patterns: &JustPatterns,
    source: &str,
    raw_line: &str,
    offset: usize,
) -> Option<(String, SourceSpan)> {
    (!state.consumes_multiline_string(raw_line)).then_some(())?;
    let authored_line = raw_line.trim_end_matches(['\r', '\n']);
    let authored_indentation = authored_line.len() - authored_line.trim_start().len();
    let authored_trimmed = authored_line.trim_start();
    let continues = trailing_continuation(authored_line);
    (!state.consumes_recipe_prefix(authored_trimmed, authored_indentation, continues))
        .then_some(())?;
    let line = authored_line.split('#').next().unwrap_or_default();
    let indentation = line.len() - line.trim_start().len();
    let trimmed = line.trim_start();
    (!trimmed.is_empty()).then_some(())?;
    (!state.consumes_top_level(trimmed, indentation, &patterns.recipe_header)).then_some(())?;
    (state.recipe_body && !state.script_recipe).then_some(())?;
    if continues {
        state.continued_recipe_line = true;
        return None;
    }
    refinery_command_on_line(patterns, source, raw_line, trimmed, indentation, offset)
}

fn trailing_continuation(line: &str) -> bool {
    line.trim_end()
        .chars()
        .rev()
        .take_while(|character| *character == '\\')
        .count()
        % 2
        == 1
}

fn refinery_command_on_line(
    patterns: &JustPatterns,
    source: &str,
    raw_line: &str,
    trimmed: &str,
    indentation: usize,
    offset: usize,
) -> Option<(String, SourceSpan)> {
    let (recipe, command_prefix) = trimmed
        .strip_prefix('@')
        .map_or((trimmed, 0), |recipe| (recipe, 1));
    let command = patterns
        .command
        .find(recipe)
        .filter(|matched| matched.start() == 0)?;
    let arguments = patterns.path_argument.captures(&recipe[command.end()..])?;
    let directory = arguments
        .get(1)
        .or_else(|| arguments.get(2))
        .or_else(|| arguments.get(3))
        .and_then(|matched| portable_migration_directory(matched.as_str()))?;
    let end = offset + raw_line.trim_end_matches(['\r', '\n']).len();
    Some((
        directory,
        source_span(source, offset + indentation + command_prefix, end),
    ))
}

fn nonempty_toml_array_setting(source: &str, section: &str, setting: &str) -> bool {
    let mut current_section = "";
    let mut value = String::new();
    let mut collecting = false;
    for raw_line in source.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') && line.ends_with(']') && !collecting {
            current_section = line[1..line.len() - 1].trim();
            continue;
        }
        if current_section != section {
            continue;
        }
        if !collecting {
            let Some((key, remainder)) = line.split_once('=') else {
                continue;
            };
            if key.trim() != setting {
                continue;
            }
            value.push_str(remainder.trim());
            collecting = true;
        } else {
            value.push_str(line);
        }
        if value.contains(']') {
            return value
                .strip_prefix('[')
                .and_then(|value| value.split_once(']'))
                .is_some_and(|(members, _)| {
                    members
                        .chars()
                        .any(|character| matches!(character, '\'' | '"'))
                });
        }
    }
    false
}

fn sqlx_integrity(
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
) -> MigrationIntegrity {
    let configuration_path = implementation.runtime_manifests.iter().find(|path| {
        path.ends_with("sqlx.toml")
            && manifests.get(*path).is_some_and(|source| {
                nonempty_toml_array_setting(source, "migrate", "ignored-chars")
            })
    });
    MigrationIntegrity {
        mechanism: "sqlx_content_checksum".into(),
        content_normalization: if configuration_path.is_some() {
            "configured_ignored_characters".into()
        } else {
            "byte_exact".into()
        },
        configuration_path: configuration_path.cloned(),
    }
}

fn configured_rust_migration_directories(
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
    framework: &str,
) -> (BTreeSet<String>, BTreeMap<String, ArtifactDeclaration>) {
    let Some(dependency) = rust_migration_dependency(framework) else {
        return (BTreeSet::new(), BTreeMap::new());
    };
    if !implementation_declares_dependency(manifests, implementation, dependency) {
        return (BTreeSet::new(), BTreeMap::new());
    }
    let mut directories = BTreeSet::from(["migrations".to_string()]);
    let mut declarations = BTreeMap::new();
    if let Some(declaration) = cargo_dependency_declaration(manifests, implementation, dependency) {
        declarations.insert("migrations".to_string(), declaration);
    }
    collect_sqlx_manifest_directory(
        manifests,
        implementation,
        framework,
        &mut directories,
        &mut declarations,
    );
    collect_rust_source_directories(
        files,
        implementation,
        framework,
        &mut directories,
        &mut declarations,
    );
    collect_refinery_justfile_directories(
        manifests,
        implementation,
        framework,
        &mut directories,
        &mut declarations,
    );
    (directories, declarations)
}

fn rust_migration_dependency(framework: &str) -> Option<&'static str> {
    match framework {
        "diesel" => Some("diesel_migrations"),
        "refinery" => Some("refinery"),
        "sqlx" => Some("sqlx"),
        _ => None,
    }
}

fn rust_migration_path_patterns(framework: &str) -> &'static [&'static str] {
    match framework {
        "sqlx" => &[
            r#"(?:Migrator::new|sqlx::migrate::Migrator::new)\s*\(\s*(?:Path::new\s*\(\s*)?[\"']([^\"']+)[\"']"#,
            r#"(?:sqlx(?:::[A-Za-z_][A-Za-z0-9_]*)*::)?migrate!\s*\(\s*[\"']([^\"']+)[\"']"#,
        ],
        "diesel" => &[
            r#"FileBasedMigrations::from_path\s*\(\s*(?:Path::new\s*\(\s*)?[\"']([^\"']+)[\"']"#,
            r#"(?:diesel_migrations(?:::[A-Za-z_][A-Za-z0-9_]*)*::)?embed_migrations!\s*\(\s*[\"']([^\"']+)[\"']"#,
        ],
        "refinery" => &[
            r#"(?:refinery(?:::[A-Za-z_][A-Za-z0-9_]*)*::)?load_sql_migrations\s*\(\s*(?:Path::new\s*\(\s*)?[\"']([^\"']+)[\"']"#,
            r#"(?:refinery(?:::[A-Za-z_][A-Za-z0-9_]*)*::)?embed_migrations!\s*\(\s*[\"']([^\"']+)[\"']"#,
        ],
        _ => &[],
    }
}

fn collect_sqlx_manifest_directory(
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
    framework: &str,
    directories: &mut BTreeSet<String>,
    declarations: &mut BTreeMap<String, ArtifactDeclaration>,
) {
    if framework != "sqlx" {
        return;
    }
    for path in &implementation.runtime_manifests {
        let configured = path
            .ends_with("sqlx.toml")
            .then(|| manifests.get(path))
            .flatten();
        if let Some((directory, span)) =
            configured.and_then(|source| literal_toml_setting(source, "migrate", "migrations-dir"))
        {
            directories.insert(directory.clone());
            declarations.insert(
                directory,
                ArtifactDeclaration {
                    path: path.clone(),
                    span,
                },
            );
        }
    }
}

fn collect_rust_source_directories(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
    framework: &str,
    directories: &mut BTreeSet<String>,
    declarations: &mut BTreeMap<String, ArtifactDeclaration>,
) {
    let patterns = rust_migration_path_patterns(framework);
    for path in &implementation.source_files {
        let Some(source) = files.get(path) else {
            continue;
        };
        let Some(code) = code_only(path, source) else {
            continue;
        };
        for pattern in patterns {
            let expression = Regex::new(pattern).expect("static Rust migration path regex");
            for capture in expression.captures_iter(source) {
                let matched = capture.get(0).expect("Rust migration path match");
                if code[matched.range()].trim().is_empty() {
                    continue;
                }
                if let Some(directory) = portable_migration_directory(&capture[1]) {
                    directories.insert(directory.clone());
                    declarations.insert(
                        directory,
                        ArtifactDeclaration {
                            path: path.clone(),
                            span: source_span(source, matched.start(), matched.end()),
                        },
                    );
                }
            }
        }
    }
}

fn collect_refinery_justfile_directories(
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
    framework: &str,
    directories: &mut BTreeSet<String>,
    declarations: &mut BTreeMap<String, ArtifactDeclaration>,
) {
    if framework != "refinery" {
        return;
    }
    for path in implementation
        .runtime_manifests
        .iter()
        .filter(|path| is_justfile(path))
    {
        let Some(source) = manifests.get(path) else {
            continue;
        };
        for (directory, span) in refinery_cli_directories(source) {
            directories.insert(directory.clone());
            declarations.insert(
                directory,
                ArtifactDeclaration {
                    path: path.clone(),
                    span,
                },
            );
        }
    }
}

fn is_justfile(path: &str) -> bool {
    matches!(
        Path::new(path).file_name().and_then(|name| name.to_str()),
        Some("Justfile" | "justfile")
    )
}

fn indexed_directory_owns_path(
    directories: &BTreeMap<String, BTreeSet<String>>,
    implementation: &Implementation,
    path: &str,
) -> bool {
    let target = implementation_relative_path(implementation, path);
    directories
        .get(&implementation.id)
        .is_some_and(|directories| {
            directories
                .iter()
                .any(|directory| Path::new(target).starts_with(directory))
        })
}

fn indexed_directory_declaration<'a>(
    index: &'a MigrationArtifactIndex,
    implementation: &Implementation,
    framework: &str,
    path: &str,
) -> Option<&'a ArtifactDeclaration> {
    let target = implementation_relative_path(implementation, path);
    index
        .directory_declarations
        .iter()
        .filter(|((implementation_id, indexed_framework, directory), _)| {
            implementation_id == &implementation.id
                && indexed_framework == framework
                && Path::new(target).starts_with(directory)
        })
        .max_by_key(|((_, _, directory), _)| Path::new(directory).components().count())
        .map(|(_, declaration)| declaration)
}

fn attach_declaration(
    mut artifact: MigrationArtifact,
    declaration: Option<&ArtifactDeclaration>,
) -> MigrationArtifact {
    if let Some(declaration) = declaration {
        artifact.declaration_path = Some(declaration.path.clone());
        artifact.declaration_span = Some(declaration.span.clone());
    }
    artifact
}

pub fn migration_artifact_index(
    rule_pack: &str,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    implementations: &[Implementation],
) -> MigrationArtifactIndex {
    let mut index = MigrationArtifactIndex::default();
    match rule_pack {
        "typescript-v1" => {
            index_typescript_migration_artifacts(&mut index, files, manifests, implementations)
        }
        "rust-v1" => index_rust_migration_artifacts(&mut index, files, manifests, implementations),
        _ => {}
    }
    index
}

fn index_typescript_migration_artifacts(
    index: &mut MigrationArtifactIndex,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    implementations: &[Implementation],
) {
    for implementation in implementations {
        index_drizzle_configuration(index, files, implementation);
        index_prisma_7_configuration(index, files, implementation);
        index_prisma_8_configuration(index, files, manifests, implementation);
    }
}

fn index_drizzle_configuration(
    index: &mut MigrationArtifactIndex,
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
) {
    let Some(configuration) = drizzle_directories(files, implementation) else {
        return;
    };
    index
        .drizzle_directories
        .insert(implementation.id.clone(), configuration.directories);
    index_directory_declarations(
        index,
        implementation,
        "drizzle_kit",
        configuration.declarations,
    );
}

fn index_prisma_7_configuration(
    index: &mut MigrationArtifactIndex,
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
) {
    let (directories, declarations) = prisma_7_directories(files, implementation);
    if directories.is_empty() {
        return;
    }
    index
        .prisma_7_directories
        .insert(implementation.id.clone(), directories);
    index_directory_declarations(index, implementation, "prisma_7", declarations);
}

fn index_prisma_8_configuration(
    index: &mut MigrationArtifactIndex,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
) {
    if !implementation_has_prisma_8_rc(manifests, implementation) {
        return;
    }
    let mut directories = BTreeSet::new();
    let mut declarations = BTreeMap::new();
    for path in &implementation.source_files {
        if let Some((directory, declaration)) =
            prisma_8_source_directory(files, implementation, path)
        {
            directories.insert(directory.clone());
            declarations.insert(directory, declaration);
        }
    }
    if directories.is_empty() {
        return;
    }
    index
        .prisma_8_directories
        .insert(implementation.id.clone(), directories);
    index_directory_declarations(index, implementation, "prisma_8", declarations);
}

fn prisma_8_source_directory(
    files: &BTreeMap<String, String>,
    implementation: &Implementation,
    path: &str,
) -> Option<(String, ArtifactDeclaration)> {
    let source = files.get(path)?;
    let imports = actual_imports(path, source)?;
    let class = typescript_class_shapes(source)?
        .into_iter()
        .find(|class| prisma_class_shell(class, &imports, false))?;
    let directory = implementation_relative_path(implementation, path)
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory)
        .to_string();
    Some((
        directory,
        ArtifactDeclaration {
            path: path.to_string(),
            span: class.span,
        },
    ))
}

fn index_rust_migration_artifacts(
    index: &mut MigrationArtifactIndex,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    implementations: &[Implementation],
) {
    for implementation in implementations {
        for framework in ["diesel", "refinery", "sqlx"] {
            index_rust_framework(index, files, manifests, implementation, framework);
        }
    }
}

fn index_rust_framework(
    index: &mut MigrationArtifactIndex,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
    framework: &str,
) {
    let (directories, declarations) =
        configured_rust_migration_directories(files, manifests, implementation, framework);
    if directories.is_empty() {
        return;
    }
    rust_directory_index(index, framework)
        .expect("known Rust migration framework")
        .insert(implementation.id.clone(), directories);
    index_directory_declarations(index, implementation, framework, declarations);
}

fn rust_directory_index<'a>(
    index: &'a mut MigrationArtifactIndex,
    framework: &str,
) -> Option<&'a mut BTreeMap<String, BTreeSet<String>>> {
    match framework {
        "diesel" => Some(&mut index.diesel_directories),
        "refinery" => Some(&mut index.refinery_directories),
        "sqlx" => Some(&mut index.sqlx_directories),
        _ => None,
    }
}

fn index_directory_declarations(
    index: &mut MigrationArtifactIndex,
    implementation: &Implementation,
    framework: &str,
    declarations: BTreeMap<String, ArtifactDeclaration>,
) {
    for (directory, declaration) in declarations {
        index.directory_declarations.insert(
            (implementation.id.clone(), framework.to_string(), directory),
            declaration,
        );
    }
}

fn typescript_migration_artifact_for(
    index: &MigrationArtifactIndex,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    prisma_7_configured_artifact(index, implementation, path)
        .or_else(|| prisma_8_artifact(index, implementation, path))
        .or_else(|| prisma_7_default_artifact(implementation, path))
        .or_else(|| drizzle_artifact(index, implementation, path))
}

fn prisma_7_configured_artifact(
    index: &MigrationArtifactIndex,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    static PRISMA_CONFIGURED_SQL: OnceLock<Regex> = OnceLock::new();
    let target = implementation_relative_path(implementation, path);
    let configured = index
        .prisma_7_directories
        .get(&implementation.id)
        .is_some_and(|directories| {
            directories
                .iter()
                .any(|directory| Path::new(target).starts_with(directory))
        });
    (configured
        && matches(
            path,
            &PRISMA_CONFIGURED_SQL,
            r"(?:^|/)[^/]+/migration\.sql$",
        ))
    .then(|| {
        attach_declaration(
            artifact(
                "prisma_7",
                "prisma:7-configured-path",
                &implementation.id,
                path,
                [
                    "configuration_path:prisma_config",
                    "canonical_filename:migration_sql",
                ],
            ),
            indexed_directory_declaration(index, implementation, "prisma_7", path),
        )
    })
}

fn prisma_8_artifact(
    index: &MigrationArtifactIndex,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    let target = implementation_relative_path(implementation, path);
    let owned = index
        .prisma_8_directories
        .get(&implementation.id)
        .is_some_and(|directories| {
            directories
                .iter()
                .any(|directory| Path::new(target).starts_with(directory))
        });
    let bundle = if path.ends_with("/ops.json") {
        bundle_artifact(
            "database_migration_operations",
            "prisma_8",
            &implementation.id,
            path,
            "migration_operations_not_analyzed",
            "prisma:8-rc-ops",
            ["framework_source:prisma_8", "bundle_member:ops_json"],
        )
    } else if path.ends_with("/migration.json") {
        bundle_artifact(
            "database_migration_metadata",
            "prisma_8",
            &implementation.id,
            path,
            "migration_metadata_inventoried",
            "prisma:8-rc-metadata",
            ["framework_source:prisma_8", "bundle_member:migration_json"],
        )
    } else {
        return None;
    };
    owned.then(|| {
        attach_declaration(
            bundle,
            indexed_directory_declaration(index, implementation, "prisma_8", path),
        )
    })
}

fn prisma_7_default_artifact(
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    static PRISMA_DEFAULT_SQL: OnceLock<Regex> = OnceLock::new();
    matches(
        path,
        &PRISMA_DEFAULT_SQL,
        r"(?:^|/)prisma/migrations/[^/]+/migration\.sql$",
    )
    .then(|| {
        artifact(
            "prisma_7",
            "prisma:7-default-path",
            &implementation.id,
            path,
            [
                "canonical_path:prisma_migrate",
                "canonical_filename:migration_sql",
            ],
        )
    })
}

fn drizzle_artifact(
    index: &MigrationArtifactIndex,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    static VERSIONED_SQL: OnceLock<Regex> = OnceLock::new();
    static DRIZZLE_V1_SQL: OnceLock<Regex> = OnceLock::new();
    if !drizzle_owns_path(index, implementation, path) {
        return None;
    }
    if matches(
        path,
        &DRIZZLE_V1_SQL,
        r"(?:^|/)[0-9]{14}_[^/]+/migration\.sql$",
    ) {
        return Some(attach_declaration(
            artifact(
                "drizzle_kit",
                "drizzle:v1",
                &implementation.id,
                path,
                ["framework_source:drizzle", "layout:drizzle_v1"],
            ),
            indexed_directory_declaration(index, implementation, "drizzle_kit", path),
        ));
    }
    matches(path, &VERSIONED_SQL, r"(?:^|/)[0-9]+_[^/]+\.sql$").then(|| {
        attach_declaration(
            artifact(
                "drizzle_kit",
                "drizzle:legacy-flat",
                &implementation.id,
                path,
                ["framework_source:drizzle", "versioned_sql_filename"],
            ),
            indexed_directory_declaration(index, implementation, "drizzle_kit", path),
        )
    })
}

fn rust_migration_artifact_for(
    index: &MigrationArtifactIndex,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    static DIESEL_SQL: OnceLock<Regex> = OnceLock::new();
    static REFINERY_SQL: OnceLock<Regex> = OnceLock::new();
    static VERSIONED_SQL: OnceLock<Regex> = OnceLock::new();
    if (diesel_embeds_path(files, implementation, path)
        || indexed_directory_owns_path(&index.diesel_directories, implementation, path))
        && matches(path, &DIESEL_SQL, r"(?:^|/)[0-9][^/]*/(?:up|down)\.sql$")
    {
        return Some(attach_declaration(
            artifact(
                "diesel",
                "diesel:file-migrations-v1",
                &implementation.id,
                path,
                [
                    "framework_source:diesel_migrations",
                    "canonical_filename:up_or_down_sql",
                ],
            ),
            indexed_directory_declaration(index, implementation, "diesel", path),
        ));
    }
    if (refinery_embeds_path(files, implementation, path)
        || indexed_directory_owns_path(&index.refinery_directories, implementation, path))
        && matches(path, &REFINERY_SQL, r"(?:^|/)[UV][0-9]+__[^/]+\.sql$")
    {
        return Some(attach_declaration(
            artifact(
                "refinery",
                "refinery:versioned-sql-v1",
                &implementation.id,
                path,
                ["framework_source:refinery", "versioned_sql_filename"],
            ),
            indexed_directory_declaration(index, implementation, "refinery", path),
        ));
    }
    if (sqlx_embeds_path(files, implementation, path)
        || indexed_directory_owns_path(&index.sqlx_directories, implementation, path))
        && matches(
            path,
            &VERSIONED_SQL,
            r"(?:^|/)[0-9]+_[^/]+(?:\.(?:up|down))?\.sql$",
        )
    {
        let mut artifact = artifact(
            "sqlx",
            "sqlx:file-migrations-v1",
            &implementation.id,
            path,
            ["embedding_source:sqlx_migrate", "versioned_sql_filename"],
        );
        artifact.integrity = Some(sqlx_integrity(manifests, implementation));
        return Some(attach_declaration(
            artifact,
            indexed_directory_declaration(index, implementation, "sqlx", path),
        ));
    }
    None
}

fn migration_artifact_for(
    rule_pack: &str,
    files: &BTreeMap<String, String>,
    manifests: &BTreeMap<String, String>,
    index: &MigrationArtifactIndex,
    implementation: &Implementation,
    path: &str,
) -> Option<MigrationArtifact> {
    match rule_pack {
        "typescript-v1" => typescript_migration_artifact_for(index, implementation, path),
        "rust-v1" => rust_migration_artifact_for(index, files, manifests, implementation, path),
        _ => None,
    }
}

pub fn migration_artifact_path_candidate(rule_pack: &str, path: &str) -> bool {
    static VERSIONED_SQL: OnceLock<Regex> = OnceLock::new();
    static DRIZZLE_V1_SQL: OnceLock<Regex> = OnceLock::new();
    static PRISMA_SQL: OnceLock<Regex> = OnceLock::new();
    static PRISMA_8_BUNDLE: OnceLock<Regex> = OnceLock::new();
    static DIESEL_SQL: OnceLock<Regex> = OnceLock::new();
    static REFINERY_SQL: OnceLock<Regex> = OnceLock::new();
    match rule_pack {
        "typescript-v1" => {
            matches(path, &VERSIONED_SQL, r"(?:^|/)[0-9]+_[^/]+\.sql$")
                || matches(
                    path,
                    &DRIZZLE_V1_SQL,
                    r"(?:^|/)[0-9]{14}_[^/]+/migration\.sql$",
                )
                || matches(path, &PRISMA_SQL, r"(?:^|/)[^/]+/migration\.sql$")
                || matches(
                    path,
                    &PRISMA_8_BUNDLE,
                    r"(?:^|/)[0-9]{8}T[0-9]{4}_[^/]+/(?:ops|migration)\.json$",
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
    manifests: &BTreeMap<String, String>,
    index: &MigrationArtifactIndex,
    implementations: &[Implementation],
    path: &str,
) -> bool {
    implementation_for_artifact(implementations, path).is_some_and(|implementation| {
        migration_artifact_for(rule_pack, files, manifests, index, implementation, path).is_some()
    })
}

pub fn resolve_migration_artifacts(input: &Input) -> Vec<MigrationArtifact> {
    let index = migration_artifact_index(
        &input.policy.rule_pack,
        &input.files,
        &input.manifests,
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
                &input.manifests,
                &index,
                implementation,
                path,
            )
        })
        .collect()
}

pub fn resolve_migration_artifact_candidates(input: &Input) -> Vec<MigrationArtifactCandidate> {
    input
        .artifact_candidates
        .keys()
        .map(|path| {
            let matched_convention = if input.policy.rule_pack == "typescript-v1" {
                if path.ends_with("/ops.json") || path.ends_with("/migration.json") {
                    "prisma_8_bundle_member"
                } else if path.ends_with("/migration.sql") {
                    "typescript_migration_sql"
                } else {
                    "typescript_versioned_sql"
                }
            } else if path.ends_with("/up.sql") || path.ends_with("/down.sql") {
                "diesel_versioned_directory_sql"
            } else if path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with(['U', 'V']))
            {
                "refinery_versioned_sql"
            } else {
                "sqlx_versioned_sql"
            };
            MigrationArtifactCandidate {
                role: "possible_database_migration_artifact".into(),
                path: path.clone(),
                matched_convention: matched_convention.into(),
                evidence_sought: vec!["framework_configuration_or_runtime_path".into()],
                unresolved_reason: "framework_ownership_not_resolved".into(),
            }
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
            let imports = actual_imports(path, source).unwrap_or_default();
            let package_signals: &[(&str, &str)] = match input.policy.rule_pack.as_str() {
                "python-v1" => &[("alembic", "alembic"), ("django.db", "django_migrations")],
                "typescript-v1" => &[
                    ("typeorm", "typeorm"),
                    ("@mikro-orm/migrations", "mikro_orm_migrations"),
                    ("kysely", "kysely"),
                    ("knex", "knex"),
                    ("umzug", "umzug"),
                    ("sequelize", "sequelize"),
                    ("@prisma/orm-postgres", "prisma_8_postgres_migration"),
                    ("@prisma/orm-mongo", "prisma_8_mongo_migration"),
                ],
                "rust-v1" => &[
                    ("sea_orm_migration", "sea_orm_migration"),
                    ("refinery", "refinery"),
                    ("rusqlite_migration", "rusqlite_migration"),
                ],
                _ => &[],
            };
            for (package, label) in package_signals {
                let present = match input.policy.rule_pack.as_str() {
                    "python-v1" => python_imports(&imports, package),
                    "typescript-v1" => imported_from(&imports, package),
                    "rust-v1" => code_only(path, source).is_some_and(|code| code.contains(package)),
                    _ => false,
                };
                if present {
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

    fn implementation(id: &str, root: Option<&str>, source_files: &[&str]) -> Implementation {
        Implementation {
            id: id.into(),
            root: root.map(str::to_string),
            ownership: "manifest",
            implementation_types: Vec::new(),
            runtime_types: Vec::new(),
            runtime_manifests: Vec::new(),
            source_files: source_files.iter().map(|path| (*path).into()).collect(),
        }
    }

    fn policy(rule_pack: &str) -> Policy {
        Policy {
            schema_version: 1,
            rule_pack: rule_pack.into(),
            scanner_version: env!("CARGO_PKG_VERSION").into(),
            scope: "repository".into(),
            default_groups: vec!["all".into()],
            exclude_directories: Vec::new(),
            source_roles: Vec::new(),
            limits: crate::policy::Limits {
                maximum_files: 100,
                maximum_pairs: 100,
                maximum_group_combinations: 100,
            },
            rules: BTreeMap::new(),
            exceptions: Vec::new(),
            resolved: crate::policy::ResolvedSelection::default(),
        }
    }

    fn input(rule_pack: &str, files: BTreeMap<String, String>) -> Input {
        Input {
            files,
            manifests: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            artifact_candidates: BTreeMap::new(),
            implementations: Vec::new(),
            source_roles: Vec::new(),
            history: crate::core::model::History::default(),
            policy: policy(rule_pack),
            digest: "0".repeat(64),
            mode: "test",
        }
    }

    #[test]
    fn package_import_matching_accepts_direct_subpath_and_require_forms_only() {
        for statement in [
            "import { MigrationInterface } from 'typeorm';",
            "import { migrate } from 'drizzle-orm/node-postgres/migrator';",
            "const knex = require('knex');",
        ] {
            let package = if statement.contains("typeorm") {
                "typeorm"
            } else if statement.contains("drizzle") {
                "drizzle-orm"
            } else {
                "knex"
            };
            assert!(imported_from(&[statement.into()], package));
        }
        assert!(!imported_from(
            &["import { MigrationInterface } from 'another-package';".into()],
            "typeorm"
        ));
        assert!(!imported_from(&[], "typeorm"));
    }

    #[test]
    fn exported_typescript_migration_functions_require_an_exact_supported_name() {
        assert!(exported_typescript_function(
            "export async function up(db: unknown) {}",
            "up"
        ));
        assert!(exported_typescript_function(
            "export const down: Migration = async () => {};",
            "down"
        ));
        assert!(!exported_typescript_function(
            "async function up(db: unknown) {}",
            "up"
        ));
        assert!(!exported_typescript_function(
            "export function upgrade() {}",
            "up"
        ));
        assert!(!exported_typescript_function(
            "export function up() {}",
            "apply"
        ));
    }

    #[test]
    fn default_typescript_imports_apply_only_to_the_migration_contract() {
        let imports = vec!["import AppMigration from 'umzug';".into()];
        assert_eq!(
            typescript_imported_names(&imports, "umzug", "Migration"),
            BTreeSet::from(["AppMigration".into()])
        );
        assert!(typescript_imported_names(&imports, "umzug", "QueryInterface").is_empty());
    }

    #[test]
    fn umzug_annotations_cannot_satisfy_other_export_contracts() {
        let code = "export const up: Migration = async (context: unknown) => {};";
        let imports = vec!["import type { Migration } from 'umzug';".into()];
        assert!(typescript_export_parameter_matches(
            TypeScriptExportContract::SequelizeUmzug,
            code,
            &imports,
            "up",
            false,
        ));
        assert!(!typescript_export_parameter_matches(
            TypeScriptExportContract::Knex,
            code,
            &imports,
            "up",
            false,
        ));
        assert!(!typescript_export_parameter_matches(
            TypeScriptExportContract::SequelizeUmzug,
            "export async function up(context: unknown) {}",
            &[],
            "up",
            false,
        ));
    }

    #[test]
    fn unrelated_sequelize_import_does_not_replace_umzug_framework_evidence() {
        let code = concat!(
            "import { Migration, SequelizeStorage } from 'umzug';\n",
            "import { DataTypes } from 'sequelize';\n",
            "const storage = new SequelizeStorage({ sequelize });\n",
            "export const up: Migration = async ({ context }) => {};\n",
            "export const down: Migration = async ({ context }) => {};\n",
        );
        let imports = actual_imports("migration.ts", code).unwrap();

        assert_eq!(
            inferred_sequelize_umzug(&imports, code, false),
            Some("framework_import:umzug")
        );
    }

    #[test]
    fn umzug_database_alias_requires_the_complete_bootstrap_contract() {
        let implementation = implementation("app", Some("."), &["db/migrate.ts"]);
        let complete = concat!(
            "import { SequelizeStorage, Umzug } from 'umzug';\n",
            "const umzug = new Umzug({ storage: new SequelizeStorage({ sequelize }) });\n",
            "export type Migration = typeof umzug._types.migration;\n",
        );
        let modules = |source: &str| {
            implementation_database_umzug_alias_modules(
                &BTreeMap::from([("db/migrate.ts".into(), source.into())]),
                &implementation,
            )
        };

        assert_eq!(modules(complete), BTreeSet::from(["db/migrate".into()]));
        for incomplete in [
            complete.replace("new Umzug", "createUmzug"),
            complete.replace("new SequelizeStorage", "createStorage"),
            complete.replace("export type Migration", "type Migration"),
            "export type Migration = typeof umzug._types.migration;\n".into(),
        ] {
            assert!(modules(&incomplete).is_empty());
        }
    }

    #[test]
    fn local_typescript_module_resolution_stays_within_relative_posix_paths() {
        assert_eq!(
            resolve_relative_typescript_module("db/migrations/001.ts", "../migrate"),
            Some("db/migrate".into())
        );
        assert_eq!(
            resolve_relative_typescript_module("db/migrations/001.ts", "./types.ts"),
            Some("db/migrations/types".into())
        );
        assert_eq!(
            resolve_relative_typescript_module("db/migrations/001.ts", "db/migrate"),
            None
        );
        assert_eq!(
            resolve_relative_typescript_module("db/migrations/001.ts", ".\\migrate"),
            None
        );
    }

    #[test]
    fn prisma_7_configuration_requires_a_literal_migrations_path() {
        let implementation = implementation("store", Some("."), &["prisma.config.ts"]);
        let configured = BTreeMap::from([(
            "prisma.config.ts".into(),
            concat!(
                "import { defineConfig } from 'prisma/config';\n",
                "export default defineConfig({\n",
                "  migrations: { path: 'database/migrations' },\n",
                "});\n",
            )
            .into(),
        )]);
        let (directories, declarations) = prisma_7_directories(&configured, &implementation);
        assert_eq!(directories, BTreeSet::from(["database/migrations".into()]));
        assert!(declarations.contains_key("database/migrations"));

        for source in [
            concat!(
                "import { defineConfig } from 'prisma/config';\n",
                "export default defineConfig({ schema: 'prisma/schema.prisma' });\n",
            ),
            concat!(
                "import { defineConfig } from 'prisma/config';\n",
                "export default defineConfig({ migrations: { path: process.env.MIGRATIONS } });\n",
            ),
        ] {
            let files = BTreeMap::from([("prisma.config.ts".into(), source.into())]);
            let (directories, declarations) = prisma_7_directories(&files, &implementation);
            assert!(directories.is_empty());
            assert!(declarations.is_empty());
        }
    }

    #[test]
    fn cargo_dependency_detection_requires_an_exact_dependency_table_or_key() {
        for manifest in [
            "[dependencies]\nsqlx = '0.8'\n",
            "[workspace.dependencies]\nsqlx = '0.8'\n",
            "[target.'cfg(unix)'.dependencies]\nsqlx = '0.8'\n",
            "[dependencies.sqlx]\nversion = '0.8'\n",
            "[target.'cfg(unix)'.dependencies.sqlx]\nversion = '0.8'\n",
        ] {
            assert!(cargo_declares_dependency(manifest, "sqlx"));
        }

        for manifest in [
            "[package]\nsqlx = '0.8'\n",
            "[dependencies]\nsqlx-core = '0.8'\n",
            "[dependencies]\n# sqlx = '0.8'\n",
            "[features]\nsqlx = []\n",
            "[dependencies.other]\nsqlx = '0.8'\n",
        ] {
            assert!(!cargo_declares_dependency(manifest, "sqlx"));
        }

        let manifest = concat!(
            "[package]\n",
            "name = 'store'\n",
            "\n",
            "[dependencies]\n",
            "sqlx = '0.8'\n",
        );
        assert_eq!(
            cargo_dependency_span(manifest, "sqlx"),
            Some(SourceSpan {
                start_line: 5,
                start_column: 1,
                end_line: 5,
                end_column: 13,
            })
        );
    }

    #[test]
    fn rust_dependency_evidence_is_scoped_to_cargo_manifests_owned_by_the_implementation() {
        let mut implementation = implementation("store", Some("packages/store"), &[]);
        implementation.runtime_manifests = vec![
            "packages/store/Cargo.toml".into(),
            "packages/store/package.json".into(),
        ];
        let manifests = BTreeMap::from([
            (
                "packages/store/Cargo.toml".into(),
                "[dependencies]\nserde = '1'\n".into(),
            ),
            (
                "packages/store/package.json".into(),
                "[dependencies]\nsqlx = '0.8'\n".into(),
            ),
            (
                "other/Cargo.toml".into(),
                "[dependencies]\nsqlx = '0.8'\n".into(),
            ),
        ]);

        assert!(!implementation_declares_dependency(
            &manifests,
            &implementation,
            "sqlx"
        ));
        let mut owned = manifests;
        owned.insert(
            "packages/store/Cargo.toml".into(),
            "[dependencies]\nsqlx = '0.8'\n".into(),
        );
        assert!(implementation_declares_dependency(
            &owned,
            &implementation,
            "sqlx"
        ));
        let declaration = cargo_dependency_declaration(&owned, &implementation, "sqlx")
            .expect("owned Cargo dependency declaration");
        assert_eq!(declaration.path, "packages/store/Cargo.toml");
        assert_eq!(
            declaration.span,
            SourceSpan {
                start_line: 2,
                start_column: 1,
                end_line: 2,
                end_column: 13,
            }
        );
    }

    #[test]
    fn literal_toml_setting_returns_the_normalized_path_and_exact_declaration_span() {
        let source = concat!(
            "[package]\n",
            "name = 'store'\n",
            "\n",
            "[migrate]\n",
            "# The scanner ignores non-settings.\n",
            "other-setting = 'ignored'\n",
            "migrations-dir = env('MIGRATIONS')\n",
            "migrations-dir = \"./db/migrations\"\n",
        );
        assert_eq!(
            literal_toml_setting(source, "migrate", "migrations-dir"),
            Some((
                "db/migrations".into(),
                SourceSpan {
                    start_line: 8,
                    start_column: 1,
                    end_line: 8,
                    end_column: 35,
                },
            ))
        );
        assert_eq!(
            literal_toml_setting(source, "migrate", "missing-setting"),
            None
        );
    }

    #[test]
    fn toml_array_setting_requires_quoted_members_in_the_exact_section() {
        for source in [
            "[migrate]\nignored-chars = [' ', '\\n']\n",
            concat!("[migrate]\n", "ignored-chars = [\n", "  \" \" ,\n", "]\n",),
        ] {
            assert!(nonempty_toml_array_setting(
                source,
                "migrate",
                "ignored-chars"
            ));
        }

        for source in [
            "[migrate]\nignored-chars = []\n",
            "[other]\nignored-chars = [' ']\n",
            "[migrate]\nother-setting = [' ']\n",
            "[migrate]\nignored-chars = [1]\n",
            "[migrate]\nignored-chars = [ # '\"'\n]\n",
            "[migrate]\nignored-chars = ['unterminated'\n",
        ] {
            assert!(!nonempty_toml_array_setting(
                source,
                "migrate",
                "ignored-chars"
            ));
        }
    }

    #[test]
    fn sqlx_integrity_requires_configuration_content_in_an_owned_sqlx_toml() {
        let mut implementation = implementation("store", Some("packages/store"), &[]);
        implementation.runtime_manifests = vec![
            "packages/store/Cargo.toml".into(),
            "packages/store/sqlx.toml".into(),
        ];
        let configured_content = "[migrate]\nignored-chars = [' ']\n";
        let mut manifests = BTreeMap::from([
            (
                "packages/store/Cargo.toml".into(),
                configured_content.into(),
            ),
            (
                "packages/store/sqlx.toml".into(),
                "[migrate]\nignored-chars = []\n".into(),
            ),
        ]);

        assert_eq!(
            sqlx_integrity(&manifests, &implementation),
            MigrationIntegrity {
                mechanism: "sqlx_content_checksum".into(),
                content_normalization: "byte_exact".into(),
                configuration_path: None,
            }
        );
        manifests.insert("packages/store/sqlx.toml".into(), configured_content.into());
        assert_eq!(
            sqlx_integrity(&manifests, &implementation),
            MigrationIntegrity {
                mechanism: "sqlx_content_checksum".into(),
                content_normalization: "configured_ignored_characters".into(),
                configuration_path: Some("packages/store/sqlx.toml".into()),
            }
        );
    }

    #[test]
    fn rust_migration_contract_tables_cover_each_supported_framework() {
        for (framework, dependency) in [
            ("sqlx", "sqlx"),
            ("diesel", "diesel_migrations"),
            ("refinery", "refinery"),
        ] {
            assert_eq!(rust_migration_dependency(framework), Some(dependency));
            assert_eq!(rust_migration_path_patterns(framework).len(), 2);
        }
        assert_eq!(rust_migration_dependency("unknown"), None);
        assert!(rust_migration_path_patterns("unknown").is_empty());
    }

    #[test]
    fn justfile_detection_accepts_only_the_exact_supported_basenames() {
        for path in ["Justfile", "justfile", "packages/store/Justfile"] {
            assert!(is_justfile(path));
        }
        for path in [
            "Cargo.toml",
            "JUSTFILE",
            "Justfile.toml",
            "packages/store/myjustfile",
            "packages/store/",
        ] {
            assert!(!is_justfile(path));
        }
    }

    #[test]
    fn indexed_directory_declarations_match_implementation_framework_and_deepest_path() {
        let implementation = implementation("store", Some("packages/store"), &[]);
        let declaration = |path: &str| ArtifactDeclaration {
            path: path.into(),
            span: SourceSpan {
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 2,
            },
        };
        let index = MigrationArtifactIndex {
            directory_declarations: BTreeMap::from([
                (
                    ("store".into(), "sqlx".into(), "db/migrations".into()),
                    declaration("packages/store/sqlx.toml"),
                ),
                (
                    (
                        "store".into(),
                        "sqlx".into(),
                        "db/migrations/special".into(),
                    ),
                    declaration("packages/store/special.toml"),
                ),
                (
                    ("other".into(), "sqlx".into(), "other/migrations".into()),
                    declaration("other/sqlx.toml"),
                ),
                (
                    ("store".into(), "diesel".into(), "diesel/migrations".into()),
                    declaration("packages/store/diesel.toml"),
                ),
            ]),
            ..MigrationArtifactIndex::default()
        };

        assert_eq!(
            indexed_directory_declaration(
                &index,
                &implementation,
                "sqlx",
                "packages/store/db/migrations/special/001.up.sql",
            )
            .map(|declaration| declaration.path.as_str()),
            Some("packages/store/special.toml")
        );
        for (framework, path) in [
            ("sqlx", "packages/store/other/migrations/001.up.sql"),
            ("diesel", "packages/store/db/migrations/001.up.sql"),
            ("sqlx", "packages/store/db/archive/001.up.sql"),
        ] {
            assert!(
                indexed_directory_declaration(&index, &implementation, framework, path).is_none()
            );
        }
    }

    #[test]
    fn portable_toml_paths_reject_empty_backslash_and_parent_traversal_values() {
        assert_eq!(
            portable_toml_path(" './db/migrations' "),
            Some("db/migrations".into())
        );
        for invalid in ["'./'", "'db\\migrations'", "'../migrations'"] {
            assert_eq!(portable_toml_path(invalid), None);
        }
    }

    #[test]
    fn portable_migration_directories_reject_absolute_and_escaping_paths() {
        assert_eq!(
            portable_migration_directory(" ./db/migrations "),
            Some("db/migrations".into())
        );
        for invalid in ["./", "/db/migrations", "db\\migrations", "../migrations"] {
            assert_eq!(portable_migration_directory(invalid), None);
        }
    }

    #[test]
    fn just_multiline_string_state_consumes_only_the_quoted_block() {
        let mut state = JustRecipeState::default();
        assert!(state.consumes_multiline_string("message := \"\"\""));
        assert!(state.consumes_multiline_string("refinery migrate -p ignored"));
        assert!(state.consumes_multiline_string("\"\"\""));
        assert!(!state.consumes_multiline_string("migrate:"));

        let mut inline = JustRecipeState::default();
        assert!(inline.consumes_multiline_string("message := '''not a recipe'''"));
        assert!(!inline.consumes_multiline_string("migrate:"));
    }

    #[test]
    fn just_recipe_prefixes_require_indentation_inside_the_recipe_body() {
        let mut continued = JustRecipeState {
            recipe_body: true,
            continued_recipe_line: true,
            ..JustRecipeState::default()
        };
        assert!(!continued.consumes_recipe_prefix("next:", 0, false));
        assert!(continued.continued_recipe_line);
        assert!(continued.consumes_recipe_prefix("continued command", 4, false));
        assert!(!continued.continued_recipe_line);

        let mut script = JustRecipeState {
            recipe_body: true,
            ..JustRecipeState::default()
        };
        assert!(!script.consumes_recipe_prefix("#!/usr/bin/env bash", 0, false));
        assert!(script.consumes_recipe_prefix("#!/usr/bin/env bash", 4, false));
        assert!(script.script_recipe);
    }

    #[test]
    fn just_top_level_state_requires_complete_attributes_and_recipe_headers() {
        let patterns = JustPatterns::new();
        let mut state = JustRecipeState::default();
        assert!(state.consumes_top_level("[script]", 0, &patterns.recipe_header));
        assert!(state.pending_script_attribute);
        assert!(!state.recipe_body);

        assert!(state.consumes_top_level("migrate:", 0, &patterns.recipe_header));
        assert!(state.recipe_body);
        assert!(state.script_recipe);
        assert!(!state.pending_script_attribute);
        assert!(!state.consumes_top_level("command", 4, &patterns.recipe_header));

        assert!(state.consumes_top_level("setting := 'value'", 0, &patterns.recipe_header));
        assert!(!state.recipe_body);
        assert!(!state.script_recipe);

        for malformed in ["[script", "script]", "[private", "private]"] {
            let mut state = JustRecipeState {
                recipe_body: true,
                script_recipe: true,
                pending_script_attribute: true,
                continued_recipe_line: true,
                ..JustRecipeState::default()
            };
            assert!(state.consumes_top_level(malformed, 0, &patterns.recipe_header));
            assert!(!state.recipe_body);
            assert!(!state.script_recipe);
            assert!(!state.pending_script_attribute);
            assert!(!state.continued_recipe_line);
        }
    }

    #[test]
    fn just_line_parser_exits_a_continued_recipe_at_the_top_level_boundary() {
        let patterns = JustPatterns::new();
        let mut state = JustRecipeState {
            recipe_body: true,
            continued_recipe_line: true,
            ..JustRecipeState::default()
        };
        let line = "setting := 'value'\n";

        assert_eq!(
            refinery_cli_directory_on_line(&mut state, &patterns, line, line, 0),
            None
        );
        assert!(!state.recipe_body);
        assert!(!state.continued_recipe_line);

        let command = "    @refinery migrate -p db/migrations\n";
        let command_source = format!("migrate:\n{command}");
        let command_offset = "migrate:\n".len();
        let mut regular = JustRecipeState {
            recipe_body: true,
            ..JustRecipeState::default()
        };
        let (directory, span) = refinery_cli_directory_on_line(
            &mut regular,
            &patterns,
            &command_source,
            command,
            command_offset,
        )
        .expect("Refinery command in a regular recipe");
        assert_eq!(directory, "db/migrations");
        assert_eq!(
            span,
            SourceSpan {
                start_line: 2,
                start_column: 6,
                end_line: 2,
                end_column: 39,
            }
        );

        let mut outside = JustRecipeState::default();
        assert_eq!(
            refinery_cli_directory_on_line(
                &mut outside,
                &patterns,
                &command_source,
                command,
                command_offset,
            ),
            None
        );
        let mut script = JustRecipeState {
            recipe_body: true,
            script_recipe: true,
            ..JustRecipeState::default()
        };
        assert_eq!(
            refinery_cli_directory_on_line(
                &mut script,
                &patterns,
                &command_source,
                command,
                command_offset,
            ),
            None
        );

        let header = "migrate: refinery migrate -p ignored\n";
        let mut header_state = JustRecipeState::default();
        assert_eq!(
            refinery_cli_directory_on_line(&mut header_state, &patterns, header, header, 0,),
            None
        );
    }

    #[test]
    fn recursive_glob_matches_zero_or_more_directories() {
        let matcher = glob_regex("/backend/alembic/versions/**/*.py").unwrap();
        assert!(matcher.is_match("backend/alembic/versions/one.py"));
        assert!(matcher.is_match("backend/alembic/versions/tenant/one.py"));
        assert!(!matcher.is_match("other/alembic/versions/one.py"));
    }

    #[test]
    fn source_role_globs_distinguish_segments_recursion_and_literals() {
        let direct = glob_regex("/migrations/*.rs").unwrap();
        assert!(direct.is_match("migrations/one.rs"));
        assert!(!direct.is_match("migrations/nested/one.rs"));
        assert!(!direct.is_match("migrations/oneXrs"));

        let recursive = glob_regex("/migrations/**").unwrap();
        assert!(recursive.is_match("migrations/one.rs"));
        assert!(recursive.is_match("migrations/nested/one.rs"));

        let literal = glob_regex("/migrations/v1+[draft].sql").unwrap();
        assert!(literal.is_match("migrations/v1+[draft].sql"));
        assert!(!literal.is_match("migrations/v11d.sql"));
    }

    #[test]
    fn declared_typeorm_shell_accepts_an_aliased_interface() {
        let complete = concat!(
            "import type { MigrationInterface as AppMigration, QueryRunner } from 'typeorm';\n",
            "export class Accounts implements AppMigration {\n",
            "  async up(queryRunner: QueryRunner) {}\n",
            "  async down(queryRunner: QueryRunner) {}\n",
            "}\n",
        );
        assert!(class_shell_matches(
            TypeScriptClassContract::TypeOrm,
            complete,
            "Accounts",
            true
        ));
        for incomplete in [
            complete.replace("up(queryRunner: QueryRunner)", "up()"),
            complete.replace("down(queryRunner: QueryRunner)", "down()"),
        ] {
            assert!(!class_shell_matches(
                TypeScriptClassContract::TypeOrm,
                &incomplete,
                "Accounts",
                true
            ));
        }
    }

    #[test]
    fn python_inference_requires_real_imports_and_the_complete_framework_shape() {
        let implementation = implementation("backend", Some("."), &["migration.py"]);
        let files = BTreeMap::new();
        let manifests = BTreeMap::new();
        let alembic = concat!(
            "from alembic import op\n",
            "revision = 'one'\n",
            "down_revision = None\n",
            "def upgrade():\n    op.execute('select 1')\n",
        );
        assert_eq!(
            inferred_source_role(
                "python",
                "migration.py",
                alembic,
                &files,
                &manifests,
                &implementation,
            )
            .map(|role| role.framework),
            Some("alembic".into())
        );
        for incomplete in [
            alembic.replace("down_revision", "parent_revision"),
            alembic.replace("from alembic import op", "op = 'alembic'"),
            alembic.replace("def upgrade():", "def upgrade(value):"),
        ] {
            assert!(
                inferred_source_role(
                    "python",
                    "migration.py",
                    &incomplete,
                    &files,
                    &manifests,
                    &implementation,
                )
                .is_none()
            );
        }

        let django = concat!(
            "from django.db import migrations\n",
            "class Migration(migrations.Migration):\n",
            "    dependencies = []\n",
            "    operations = []\n",
        );
        assert_eq!(
            inferred_source_role(
                "python",
                "migration.py",
                django,
                &files,
                &manifests,
                &implementation,
            )
            .map(|role| role.framework),
            Some("django".into())
        );
        for incomplete in [
            django.replace(
                "from django.db import migrations",
                "migrations = 'django.db'",
            ),
            django.replace("dependencies", "parents"),
            django.replace("operations", "steps"),
            django.replace("migrations.Migration", "object"),
        ] {
            assert!(
                inferred_source_role(
                    "python",
                    "migration.py",
                    &incomplete,
                    &files,
                    &manifests,
                    &implementation,
                )
                .is_none()
            );
        }
    }

    #[test]
    fn alembic_multidatabase_requires_both_dispatchers_and_matching_engine_entrypoints() {
        let complete = concat!(
            "def upgrade(engine_name):\n    globals()[f'upgrade_{engine_name}']()\n",
            "def downgrade(engine_name):\n    globals()[f'downgrade_{engine_name}']()\n",
            "def upgrade_primary():\n    pass\n",
            "def downgrade_primary():\n    pass\n",
        );
        assert!(alembic_multidatabase_entrypoints(complete));

        for incomplete in [
            complete.replacen("def upgrade(engine_name):", "def apply(engine_name):", 1),
            complete.replacen(
                "def downgrade(engine_name):",
                "def rollback(engine_name):",
                1,
            ),
            concat!(
                "def upgrade(engine_name):\n    pass\n",
                "def downgrade(engine_name):\n    pass\n",
            )
            .into(),
            complete.replacen("downgrade_primary", "downgrade_secondary", 1),
        ] {
            assert!(!alembic_multidatabase_entrypoints(&incomplete));
        }
    }

    #[test]
    fn django_migration_members_belong_to_the_exact_migration_class() {
        assert!(django_migration_members(concat!(
            "class Migration:\n",
            "    dependencies = []\n",
            "    operations = []\n",
        )));

        let source = concat!(
            "class HelperMigration:\n",
            "    dependencies = []\n",
            "    operations = []\n",
            "\n",
            "class Migration:\n",
            "    pass\n",
        );

        assert!(!django_migration_members(source));
    }

    #[test]
    fn declared_source_roles_resolve_provenance_and_cannot_cross_implementation_boundaries() {
        let files = BTreeMap::from([
            (
                "backend/migrations/one.py".into(),
                "def upgrade(): pass".into(),
            ),
            (
                "backend/migrations/two.py".into(),
                "def upgrade(): pass".into(),
            ),
            (
                "worker/migrations/one.py".into(),
                "def upgrade(): pass".into(),
            ),
        ]);
        let implementations = vec![
            implementation(
                "backend",
                Some("backend"),
                &["backend/migrations/one.py", "backend/migrations/two.py"],
            ),
            implementation("worker", Some("worker"), &["worker/migrations/one.py"]),
        ];
        let mut declared = policy("python-v1");
        declared.source_roles = vec![SourceRoleDeclaration {
            role: "database_migration".into(),
            framework: "alembic".into(),
            paths: vec!["/backend/migrations/*.py".into()],
            configuration: vec!["/backend/alembic.ini".into()],
        }];

        let roles = resolve_source_roles(
            &declared,
            &files,
            &BTreeMap::new(),
            "python",
            &implementations,
        )
        .unwrap();
        assert_eq!(roles.len(), 2);
        assert!(roles.iter().all(|role| role.implementation_id == "backend"));
        assert!(roles.iter().all(|role| role.source == "declared"));
        assert!(roles.iter().all(|role| {
            role.evidence.contains(
                &"policy_configuration_path_declared_unverified:/backend/alembic.ini".into(),
            )
        }));

        declared.source_roles[0].paths = vec![
            "/backend/migrations/*.py".into(),
            "/worker/migrations/*.py".into(),
        ];
        let error = resolve_source_roles(
            &declared,
            &files,
            &BTreeMap::new(),
            "python",
            &implementations,
        )
        .expect_err("one declaration cannot own files from two implementations");
        assert!(error.contains("crosses Implementation boundaries"));
        assert!(error.contains("backend"));
        assert!(error.contains("worker"));
    }

    #[test]
    fn public_role_resolution_infers_roles_and_rejects_unowned_sources() {
        let source = concat!(
            "from alembic import op\n",
            "revision = 'one'\n",
            "down_revision = None\n",
            "def upgrade():\n    op.execute('select 1')\n",
        );
        let files = BTreeMap::from([("backend/revision.py".into(), source.into())]);
        let policies = policy("python-v1");
        let implementations = vec![implementation(
            "backend",
            Some("backend"),
            &["backend/revision.py"],
        )];
        let roles = resolve_source_roles(
            &policies,
            &files,
            &BTreeMap::new(),
            "python",
            &implementations,
        )
        .unwrap();
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].framework, "alembic");
        assert_eq!(roles[0].implementation_id, "backend");
        assert_eq!(roles[0].source, "inferred");

        let error = resolve_source_roles(&policies, &files, &BTreeMap::new(), "python", &[])
            .expect_err("every source must have an implementation owner");
        assert!(error.contains("source file has no Implementation owner"));
        assert!(error.contains("backend/revision.py"));
    }

    #[test]
    fn typescript_inference_requires_each_framework_contract() {
        let cases = [
            (
                "typeorm",
                concat!(
                    "import type { MigrationInterface, QueryRunner } from 'typeorm';\n",
                    "export class Accounts implements MigrationInterface {\n",
                    "  async up(queryRunner: QueryRunner) {}\n",
                    "  async down(queryRunner: QueryRunner) {}\n",
                    "}\n",
                ),
            ),
            (
                "mikro_orm",
                concat!(
                    "import { Migration } from '@mikro-orm/migrations';\n",
                    "export class Accounts extends Migration { async up() {} }\n",
                ),
            ),
            (
                "knex",
                concat!(
                    "import type { Knex } from 'knex';\n",
                    "export async function up(db: Knex) {}\n",
                    "export async function down(db: Knex) {}\n",
                ),
            ),
            (
                "sequelize_umzug",
                concat!(
                    "import type { QueryInterface } from 'sequelize';\n",
                    "export const up = async (db: QueryInterface) => {};\n",
                    "export const down = async (db: QueryInterface) => {};\n",
                ),
            ),
        ];
        for (expected, source) in cases {
            assert_eq!(
                inferred_typescript("migration.ts", source, "frontend", &BTreeSet::new(), false,)
                    .map(|role| role.framework),
                Some(expected.into())
            );
        }

        for incomplete in [
            concat!(
                "import type { Knex } from 'knex';\n",
                "export async function up(db: Knex) {}\n",
            ),
            concat!(
                "import type { Knex } from 'knex';\n",
                "export async function up(db: Knex) {}\n",
                "export async function down(db: unknown) {}\n",
            ),
            concat!(
                "import type { MigrationInterface, QueryRunner } from 'typeorm';\n",
                "export class Accounts implements MigrationInterface {\n",
                "  async up(queryRunner: QueryRunner) {}\n",
                "}\n",
            ),
            concat!(
                "import type { MigrationInterface, QueryRunner } from 'typeorm';\n",
                "export class Accounts implements MigrationInterface {\n",
                "  async up(queryRunner: QueryRunner) {}\n",
                "  async down(queryRunner: unknown) {}\n",
                "}\n",
            ),
            concat!(
                "import { Migration } from '@mikro-orm/migrations';\n",
                "export class Accounts extends Migration { async helper() {} }\n",
            ),
            concat!(
                "import type { QueryInterface } from 'sequelize';\n",
                "export const up = async (db: QueryInterface) => {};\n",
            ),
            concat!(
                "import type { Migration } from 'umzug';\n",
                "export const up: Migration = async ({ context }) => {};\n",
                "export const down: Migration = async ({ context }) => {};\n",
            ),
            concat!(
                "import { Migration, SequelizeStorage } from 'umzug';\n",
                "const storage = new SequelizeStorage({ sequelize });\n",
                "export const up: Migration = async ({ context }) => {};\n",
                "export const down = async ({ context }: unknown) => {};\n",
            ),
        ] {
            assert!(
                inferred_typescript(
                    "migration.ts",
                    incomplete,
                    "frontend",
                    &BTreeSet::new(),
                    false,
                )
                .is_none()
            );
        }
    }

    #[test]
    fn declared_typescript_shells_still_require_the_named_class_and_canonical_members() {
        let mikro = concat!(
            "import { Migration as Base } from '@mikro-orm/migrations';\n",
            "export class Accounts extends Base { async up() {} }\n",
        );
        assert!(class_shell_matches(
            TypeScriptClassContract::MikroOrm,
            mikro,
            "Accounts",
            true
        ));
        assert!(!class_shell_matches(
            TypeScriptClassContract::MikroOrm,
            mikro,
            "Other",
            true
        ));
        assert!(!class_shell_matches(
            TypeScriptClassContract::MikroOrm,
            &mikro.replace("up", "helper"),
            "Accounts",
            true
        ));

        let prisma = concat!(
            "import { Migration as Base } from '@prisma/migrations';\n",
            "export class Accounts extends Base { get operations() { return []; } }\n",
        );
        assert!(class_shell_matches(
            TypeScriptClassContract::Prisma8,
            prisma,
            "Accounts",
            true
        ));
        assert!(!class_shell_matches(
            TypeScriptClassContract::Prisma8,
            &prisma.replace("get operations()", "operations()"),
            "Accounts",
            true
        ));
        assert!(!class_shell_matches(
            TypeScriptClassContract::Prisma8,
            "export class Accounts { get operations() { return []; } }",
            "Accounts",
            true
        ));
    }

    #[test]
    fn class_roles_allow_unimported_relationship_aliases_only_when_declared() {
        let path = "migrations/accounts.ts";
        let source = concat!(
            "export class Accounts implements AppMigration {\n",
            "  async up(context: unknown) {}\n",
            "  async down(context: unknown) {}\n",
            "}\n",
        );
        let mut input = input(
            "typescript-v1",
            BTreeMap::from([(path.into(), source.into())]),
        );
        let mut role = inferred_role("typeorm", path, "frontend", []);
        role.source = "declared".into();
        input.source_roles = vec![role];
        let location = Location {
            path: path.into(),
            line: 1,
            column: 1,
        };
        assert!(class_role(&input, path, "Accounts", &location).is_some());

        input.source_roles[0].source = "inferred".into();
        assert!(class_role(&input, path, "Accounts", &location).is_none());
    }

    #[test]
    fn sea_orm_type_roles_require_both_framework_and_migration_leaf() {
        let path = "src/m20261011_accounts.rs";
        let source = "pub struct Migration; impl MigrationTrait for Migration {}";
        let mut input = input("rust-v1", BTreeMap::from([(path.into(), source.into())]));
        input.source_roles = vec![inferred_role("sea_orm", path, "store", [])];
        let location = Location {
            path: path.into(),
            line: 1,
            column: 12,
        };

        assert!(type_role(&input, path, "store::Migration", &location).is_some());
        assert!(type_role(&input, path, "store::Other", &location).is_none());
        input.source_roles[0].framework = "refinery".into();
        assert!(type_role(&input, path, "store::Migration", &location).is_none());
    }

    #[test]
    fn canonical_functions_reject_nested_wrong_named_and_incomplete_declarations() {
        let refinery = "pub fn migration() -> String { String::new() }";
        assert!(canonical_refinery(refinery, "migration", None));
        assert!(!canonical_refinery(refinery, "helper", None));
        assert!(!canonical_refinery(refinery, "migration", Some("Module")));
        assert!(!canonical_refinery(
            "pub fn migration(value: i32) -> String { value.to_string() }",
            "migration",
            None
        ));

        let alembic = concat!(
            "revision = 'one'\ndown_revision = None\n",
            "def upgrade():\n    pass\n",
        );
        assert!(canonical_alembic(alembic, "upgrade", None));
        assert!(!canonical_alembic(alembic, "upgrade", Some("Container")));
        assert!(!canonical_alembic(
            &alembic.replace("down_revision", "parent_revision"),
            "upgrade",
            None
        ));

        let multidatabase = concat!(
            "revision = 'one'\ndown_revision = None\n",
            "def upgrade(engine_name):\n    pass\n",
            "def downgrade(engine_name):\n    pass\n",
            "def upgrade_primary():\n    pass\n",
            "def downgrade_primary():\n    pass\n",
        );
        assert!(canonical_alembic(multidatabase, "upgrade_primary", None));
        assert!(!canonical_alembic(
            concat!(
                "revision = 'one'\ndown_revision = None\n",
                "def upgrade_primary():\n    pass\n",
                "def downgrade_primary():\n    pass\n",
            ),
            "upgrade_primary",
            None
        ));
    }

    #[test]
    fn canonical_sea_orm_methods_require_the_unique_migration_trait_owner() {
        let source = concat!(
            "pub struct Migration;\n",
            "impl MigrationTrait for Migration {\n",
            "    async fn up(&self) {}\n",
            "    async fn down(&self) {}\n",
            "}\n",
        );
        assert!(canonical_sea_orm(source, "up", Some("store::Migration")));
        assert!(canonical_sea_orm(source, "down", Some("store::Migration")));
        assert!(!canonical_sea_orm(
            source,
            "helper",
            Some("store::Migration")
        ));
        assert!(!canonical_sea_orm(source, "up", None));
        assert!(!canonical_sea_orm(source, "up", Some("store::Other")));
        assert!(!canonical_sea_orm(
            "pub struct Migration;",
            "up",
            Some("store::Migration")
        ));
        assert!(!canonical_sea_orm(
            &format!("{source}\npub enum Migration {{ Duplicate }}\n"),
            "up",
            Some("store::Migration")
        ));
    }

    #[test]
    fn artifact_ownership_prefers_the_deepest_root_and_recognition_fails_closed() {
        let implementations = vec![
            implementation("repository", Some("."), &["src/lib.rs"]),
            implementation(
                "package",
                Some("packages/store"),
                &["packages/store/src/lib.rs"],
            ),
        ];
        assert_eq!(
            implementation_for_artifact(
                &implementations,
                "packages/store/migrations/V1__accounts.sql"
            )
            .map(|implementation| implementation.id.as_str()),
            Some("package")
        );
        let mut unowned = implementation("unowned", None, &[]);
        unowned.ownership = "unowned_source";
        let candidates = [implementations[1].clone(), unowned];
        let fallback =
            implementation_for_artifact(&candidates, "outside/migrations/V1__accounts.sql")
                .expect("unowned artifact fallback");
        assert_eq!(fallback.id, "unowned");
        assert_eq!(
            implementation_relative_path(
                &implementations[1],
                "packages/store/migrations/V1__accounts.sql"
            ),
            "migrations/V1__accounts.sql"
        );

        let mut files = BTreeMap::new();
        files.insert(
            "packages/store/src/lib.rs".into(),
            "use refinery::embed_migrations; embed_migrations!();".into(),
        );
        let index = MigrationArtifactIndex::default();
        let manifests = BTreeMap::new();
        assert!(is_recognized_migration_artifact(
            "rust-v1",
            &files,
            &manifests,
            &index,
            &implementations,
            "packages/store/migrations/V1__accounts.sql"
        ));
        assert!(!is_recognized_migration_artifact(
            "rust-v1",
            &files,
            &manifests,
            &index,
            &implementations,
            "packages/store/other/V1__accounts.sql"
        ));
        assert!(!is_recognized_migration_artifact(
            "unknown-v1",
            &files,
            &manifests,
            &index,
            &implementations,
            "packages/store/migrations/V1__accounts.sql"
        ));

        for (rule_pack, accepted, rejected) in [
            (
                "typescript-v1",
                "prisma/migrations/20261011_accounts/migration.sql",
                "prisma/migrations/20261011_accounts/notes.sql",
            ),
            (
                "rust-v1",
                "migrations/20261011_accounts.up.sql",
                "migrations/accounts.sql",
            ),
        ] {
            assert!(migration_artifact_path_candidate(rule_pack, accepted));
            assert!(!migration_artifact_path_candidate(rule_pack, rejected));
        }
        assert!(!migration_artifact_path_candidate(
            "python-v1",
            "migrations/20261011_accounts.sql"
        ));
    }

    #[test]
    fn refinery_embedding_accepts_explicit_and_default_directories_independently() {
        let implementation = implementation("store", Some("."), &["src/lib.rs"]);
        let explicit = BTreeMap::from([(
            "src/lib.rs".into(),
            "use refinery::embed_migrations; embed_migrations!(\"db/migrations\");".into(),
        )]);
        assert!(refinery_embeds_path(
            &explicit,
            &implementation,
            "db/migrations/V1__accounts.sql"
        ));
        assert!(!refinery_embeds_path(
            &explicit,
            &implementation,
            "migrations/V1__accounts.sql"
        ));

        let default = BTreeMap::from([(
            "src/lib.rs".into(),
            "use refinery::embed_migrations; embed_migrations!();".into(),
        )]);
        assert!(refinery_embeds_path(
            &default,
            &implementation,
            "migrations/V1__accounts.sql"
        ));

        let only_import = BTreeMap::from([(
            "src/lib.rs".into(),
            "use refinery::embed_migrations;".into(),
        )]);
        assert!(!refinery_embeds_path(
            &only_import,
            &implementation,
            "migrations/V1__accounts.sql"
        ));

        let only_invocation =
            BTreeMap::from([("src/lib.rs".into(), "embed_migrations!();".into())]);
        assert!(!refinery_embeds_path(
            &only_invocation,
            &implementation,
            "migrations/V1__accounts.sql"
        ));
    }

    #[test]
    fn sea_orm_module_paths_exclude_crate_roots_and_preserve_nested_modules() {
        let implementation = implementation("store", Some("."), &[]);
        assert_eq!(
            sea_orm_module_path(&implementation, "src/m20261011_accounts.rs").as_deref(),
            Some("m20261011_accounts")
        );
        assert_eq!(
            sea_orm_module_path(&implementation, "src/accounts/mod.rs").as_deref(),
            Some("accounts")
        );
        for path in ["src/lib.rs", "src/main.rs", "src/.rs"] {
            assert_eq!(sea_orm_module_path(&implementation, path), None);
        }
    }

    #[test]
    fn rust_inference_requires_refinery_embedding_or_seaorm_registry_membership() {
        let refinery_implementation = implementation(
            "store",
            Some("."),
            &["src/lib.rs", "migrations/V1__accounts.rs"],
        );
        let refinery_source = "pub fn migration() -> String { String::new() }";
        let mut refinery_files = BTreeMap::from([
            (
                "src/lib.rs".into(),
                "use refinery::embed_migrations; embed_migrations!();".into(),
            ),
            ("migrations/V1__accounts.rs".into(), refinery_source.into()),
        ]);
        assert_eq!(
            inferred_rust(
                "migrations/V1__accounts.rs",
                refinery_source,
                &refinery_files,
                &refinery_implementation,
            )
            .map(|role| role.framework),
            Some("refinery".into())
        );
        refinery_files.insert("src/lib.rs".into(), "fn unrelated() {}".into());
        assert!(
            inferred_rust(
                "migrations/V1__accounts.rs",
                refinery_source,
                &refinery_files,
                &refinery_implementation,
            )
            .is_none()
        );

        let seaorm_path = "migration/src/m20261011_accounts.rs";
        let seaorm_source = concat!(
            "use sea_orm_migration::prelude::*;\n",
            "#[derive(DeriveMigrationName)] struct Migration;\n",
            "impl MigrationTrait for Migration {}\n",
        );
        let seaorm_implementation = implementation(
            "migration",
            Some("migration"),
            &[seaorm_path, "migration/src/lib.rs"],
        );
        let mut seaorm_files = BTreeMap::from([
            (seaorm_path.into(), seaorm_source.into()),
            (
                "migration/src/lib.rs".into(),
                concat!(
                    "impl MigratorTrait for Migrator {\n",
                    " fn migrations() -> Vec<Box<dyn MigrationTrait>> {\n",
                    "  vec![Box::new(m20261011_accounts::Migration)]\n",
                    " }\n",
                    "}\n",
                )
                .into(),
            ),
        ]);
        assert_eq!(
            inferred_rust(
                seaorm_path,
                seaorm_source,
                &seaorm_files,
                &seaorm_implementation,
            )
            .map(|role| role.framework),
            Some("sea_orm".into())
        );
        seaorm_files.insert(
            "migration/src/lib.rs".into(),
            "impl MigratorTrait for Migrator { fn helper() {} }".into(),
        );
        assert!(
            inferred_rust(
                seaorm_path,
                seaorm_source,
                &seaorm_files,
                &seaorm_implementation,
            )
            .is_none()
        );
    }

    #[test]
    fn drizzle_configuration_accepts_only_literal_safe_directories() {
        let cases = [
            (
                "import { defineConfig as config } from 'drizzle-kit';\nconfig({ out: './db/generated' });",
                Some(BTreeSet::from(["db/generated".into()])),
            ),
            (
                "import { defineConfig } from 'drizzle-kit';\ndefineConfig({ out: '../outside' });",
                None,
            ),
            (
                "import { defineConfig } from 'drizzle-kit';\ndefineConfig({ out: directory });",
                None,
            ),
            (
                "import { defineConfig } from 'drizzle-kit';\ndefineConfig({ ...base, out: './db' });",
                None,
            ),
        ];
        for (source, expected) in cases {
            let mut configuration = DrizzleConfiguration::default();
            inspect_drizzle_source("drizzle.config.ts", source, &mut configuration);
            assert_eq!(
                resolved_drizzle_directories(configuration)
                    .map(|configuration| configuration.directories),
                expected
            );
        }

        let tsx = typescript_tree(
            "drizzle.config.tsx",
            "const preview = <section>migration</section>;",
        )
        .unwrap();
        assert!(!tsx.root_node().has_error());
    }

    #[test]
    fn unresolved_candidates_preserve_rule_pack_specific_framework_signals() {
        let cases = [
            (
                "python-v1",
                "backend/versions/one.py",
                "from alembic import op",
                "unresolved_framework_signal:alembic",
            ),
            (
                "typescript-v1",
                "frontend/migrations/one.ts",
                "import { Migration } from '@prisma/orm-postgres/migration';",
                "unresolved_framework_signal:prisma_8_postgres_migration",
            ),
            (
                "rust-v1",
                "migrations/V1__one.rs",
                "use refinery::Migration;",
                "unresolved_versioned_filename:refinery",
            ),
        ];
        for (rule_pack, path, source, expected) in cases {
            let candidates = resolve_source_role_candidates(&input(
                rule_pack,
                BTreeMap::from([(path.into(), source.into())]),
            ));
            assert_eq!(candidates.len(), 1);
            assert!(candidates[0].evidence.iter().any(|value| value == expected));
        }
        assert!(
            resolve_source_role_candidates(&input(
                "python-v1",
                BTreeMap::from([("backend/service.py".into(), "def run(): pass".into())]),
            ))
            .is_empty()
        );
    }

    #[test]
    fn unresolved_artifact_candidates_preserve_each_filename_convention() {
        for (rule_pack, path, expected) in [
            (
                "typescript-v1",
                "prisma/migrations/one/ops.json",
                "prisma_8_bundle_member",
            ),
            (
                "typescript-v1",
                "prisma/migrations/one/migration.json",
                "prisma_8_bundle_member",
            ),
            (
                "typescript-v1",
                "prisma/migrations/one/migration.sql",
                "typescript_migration_sql",
            ),
            (
                "typescript-v1",
                "drizzle/0001_accounts.sql",
                "typescript_versioned_sql",
            ),
            (
                "rust-v1",
                "migrations/one/up.sql",
                "diesel_versioned_directory_sql",
            ),
            (
                "rust-v1",
                "migrations/one/down.sql",
                "diesel_versioned_directory_sql",
            ),
            (
                "rust-v1",
                "migrations/V1__accounts.sql",
                "refinery_versioned_sql",
            ),
            (
                "rust-v1",
                "migrations/20261011_accounts.sql",
                "sqlx_versioned_sql",
            ),
        ] {
            let mut candidate_input = input(rule_pack, BTreeMap::new());
            candidate_input
                .artifact_candidates
                .insert(path.into(), Vec::new());
            let candidates = resolve_migration_artifact_candidates(&candidate_input);
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].matched_convention, expected);
        }
    }
}
