use crate::core::model::{ResolvedSourceRole, RoleContext};

pub(crate) fn leaf_symbol(symbol: &str) -> &str {
    symbol
        .rsplit([':', '.'])
        .find(|component| !component.is_empty())
        .unwrap_or(symbol)
}

pub(crate) fn function_declaration_role(framework: &str, symbol: &str) -> Option<&'static str> {
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

pub(crate) fn reported_role(
    source_role: &ResolvedSourceRole,
    declaration_role: &str,
    canonical: bool,
) -> Option<RoleContext> {
    canonical.then(|| RoleContext {
        source_role: source_role.role.clone(),
        declaration_role: declaration_role.into(),
        framework: source_role.framework.clone(),
        implementation_id: source_role.implementation_id.clone(),
        source: source_role.source.clone(),
        confidence: source_role.confidence.clone(),
        path_scope: source_role.path_scope.clone(),
        evidence: source_role.evidence.clone(),
        disposition: "reported_by_role".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_role() -> ResolvedSourceRole {
        ResolvedSourceRole {
            role: "database_migration".into(),
            framework: "alembic".into(),
            implementation_id: "backend".into(),
            path: "backend/migrations/revision.py".into(),
            path_scope: "/backend/migrations/*.py".into(),
            source: "declared".into(),
            confidence: "explicit".into(),
            evidence: vec!["policy.source_roles[0]".into()],
        }
    }

    #[test]
    fn framework_and_symbol_determine_the_function_role() {
        let cases = [
            ("alembic", "module.upgrade", "migration_up"),
            ("typeorm", "Migration::down", "migration_down"),
            ("prisma_8", "operations", "migration_operations"),
            ("refinery", "migration", "migration_sql_builder"),
        ];

        for (framework, symbol, expected) in cases {
            assert_eq!(function_declaration_role(framework, symbol), Some(expected));
        }
        assert_eq!(function_declaration_role("alembic", "helper"), None);
        assert_eq!(function_declaration_role("unknown", "up"), None);
    }

    #[test]
    fn noncanonical_declarations_are_not_role_adjusted() {
        assert_eq!(reported_role(&source_role(), "migration_up", false), None);
    }

    #[test]
    fn canonical_declarations_preserve_role_provenance_and_set_disposition() {
        let context = reported_role(&source_role(), "migration_up", true).unwrap();

        assert_eq!(context.source_role, "database_migration");
        assert_eq!(context.declaration_role, "migration_up");
        assert_eq!(context.framework, "alembic");
        assert_eq!(context.implementation_id, "backend");
        assert_eq!(context.source, "declared");
        assert_eq!(context.confidence, "explicit");
        assert_eq!(context.path_scope, "/backend/migrations/*.py");
        assert_eq!(context.evidence, ["policy.source_roles[0]"]);
        assert_eq!(context.disposition, "reported_by_role");
    }
}
