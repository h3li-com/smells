use crate::core::model::{ResolvedSourceRole, RoleContext};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TypeScriptClassContract {
    TypeOrm,
    MikroOrm,
    Prisma8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TypeScriptExportContract {
    Knex,
    Kysely,
    SequelizeUmzug,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CanonicalDeclaration {
    AlembicFunction,
    TypeScriptClass(TypeScriptClassContract),
    TypeScriptExport(TypeScriptExportContract),
    RefineryFunction,
    SeaOrmFunction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeclarationRole {
    symbol: &'static str,
    role: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CodeMigrationFramework {
    id: &'static str,
    declarations: &'static [DeclarationRole],
    canonical_declaration: CanonicalDeclaration,
}

const UP_DOWN: &[DeclarationRole] = &[
    DeclarationRole {
        symbol: "up",
        role: "migration_up",
    },
    DeclarationRole {
        symbol: "down",
        role: "migration_down",
    },
];
const ALEMBIC: &[DeclarationRole] = &[
    DeclarationRole {
        symbol: "upgrade",
        role: "migration_up",
    },
    DeclarationRole {
        symbol: "downgrade",
        role: "migration_down",
    },
];
const PRISMA_8: &[DeclarationRole] = &[DeclarationRole {
    symbol: "operations",
    role: "migration_operations",
}];
const REFINERY: &[DeclarationRole] = &[DeclarationRole {
    symbol: "migration",
    role: "migration_sql_builder",
}];

const CODE_MIGRATION_FRAMEWORKS: &[CodeMigrationFramework] = &[
    CodeMigrationFramework {
        id: "alembic",
        declarations: ALEMBIC,
        canonical_declaration: CanonicalDeclaration::AlembicFunction,
    },
    CodeMigrationFramework {
        id: "typeorm",
        declarations: UP_DOWN,
        canonical_declaration: CanonicalDeclaration::TypeScriptClass(
            TypeScriptClassContract::TypeOrm,
        ),
    },
    CodeMigrationFramework {
        id: "knex",
        declarations: UP_DOWN,
        canonical_declaration: CanonicalDeclaration::TypeScriptExport(
            TypeScriptExportContract::Knex,
        ),
    },
    CodeMigrationFramework {
        id: "mikro_orm",
        declarations: UP_DOWN,
        canonical_declaration: CanonicalDeclaration::TypeScriptClass(
            TypeScriptClassContract::MikroOrm,
        ),
    },
    CodeMigrationFramework {
        id: "kysely",
        declarations: UP_DOWN,
        canonical_declaration: CanonicalDeclaration::TypeScriptExport(
            TypeScriptExportContract::Kysely,
        ),
    },
    CodeMigrationFramework {
        id: "sequelize_umzug",
        declarations: UP_DOWN,
        canonical_declaration: CanonicalDeclaration::TypeScriptExport(
            TypeScriptExportContract::SequelizeUmzug,
        ),
    },
    CodeMigrationFramework {
        id: "prisma_8",
        declarations: PRISMA_8,
        canonical_declaration: CanonicalDeclaration::TypeScriptClass(
            TypeScriptClassContract::Prisma8,
        ),
    },
    CodeMigrationFramework {
        id: "refinery",
        declarations: REFINERY,
        canonical_declaration: CanonicalDeclaration::RefineryFunction,
    },
    CodeMigrationFramework {
        id: "sea_orm",
        declarations: UP_DOWN,
        canonical_declaration: CanonicalDeclaration::SeaOrmFunction,
    },
];

pub(crate) fn code_migration_framework(id: &str) -> Option<&'static CodeMigrationFramework> {
    CODE_MIGRATION_FRAMEWORKS
        .iter()
        .find(|framework| framework.id == id)
}

impl CodeMigrationFramework {
    pub(crate) fn canonical_declaration(&self) -> CanonicalDeclaration {
        self.canonical_declaration
    }

    pub(crate) fn declaration_role(&self, symbol: &str) -> Option<&'static str> {
        let name = leaf_symbol(symbol);
        self.declarations
            .iter()
            .find(|declaration| declaration.symbol == name)
            .map(|declaration| declaration.role)
            .or_else(|| {
                (self.id == "alembic")
                    .then(|| {
                        if name
                            .strip_prefix("upgrade_")
                            .is_some_and(|suffix| !suffix.is_empty())
                        {
                            Some("migration_up")
                        } else if name
                            .strip_prefix("downgrade_")
                            .is_some_and(|suffix| !suffix.is_empty())
                        {
                            Some("migration_down")
                        } else {
                            None
                        }
                    })
                    .flatten()
            })
    }
}

pub(crate) fn leaf_symbol(symbol: &str) -> &str {
    symbol
        .rsplit([':', '.'])
        .find(|component| !component.is_empty())
        .unwrap_or(symbol)
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
        contract_version: source_role.contract_version.clone(),
        declaration_span: source_role.declaration_span.clone(),
        path_scope: source_role.path_scope.clone(),
        evidence: source_role.evidence.clone(),
        disposition: "reported_by_role".into(),
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

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
            contract_version: "alembic:revision-v1".into(),
            configuration_status: None,
            declaration_span: None,
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
            assert_eq!(
                code_migration_framework(framework)
                    .and_then(|framework| framework.declaration_role(symbol)),
                Some(expected)
            );
        }
        assert_eq!(
            code_migration_framework("alembic")
                .and_then(|framework| framework.declaration_role("helper")),
            None
        );
        assert_eq!(code_migration_framework("unknown"), None);
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

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 128,
            max_shrink_iters: 128,
            ..ProptestConfig::default()
        })]

        #[test]
        fn namespace_qualification_does_not_change_a_known_function_role(
            prefix in "[A-Za-z_][A-Za-z0-9_]{0,20}",
            separator in prop::sample::select(vec!["::", "."]),
            (framework, symbol, expected) in prop::sample::select(vec![
                ("alembic", "upgrade", "migration_up"),
                ("alembic", "downgrade", "migration_down"),
                ("typeorm", "up", "migration_up"),
                ("typeorm", "down", "migration_down"),
                ("prisma_8", "operations", "migration_operations"),
                ("refinery", "migration", "migration_sql_builder"),
            ]),
        ) {
            let qualified = format!("{prefix}{separator}{symbol}");
            let actual = code_migration_framework(framework)
                .and_then(|framework| framework.declaration_role(&qualified));
            prop_assert_eq!(actual, Some(expected));
        }

        #[test]
        fn the_canonicality_gate_always_fails_closed(
            declaration_role in "[a-z_]{1,24}",
        ) {
            prop_assert_eq!(reported_role(&source_role(), &declaration_role, false), None);
        }

        #[test]
        fn alembic_multidatabase_suffixes_keep_their_direction(
            suffix in "[A-Za-z_][A-Za-z0-9_]{0,24}",
        ) {
            let framework = code_migration_framework("alembic").unwrap();
            prop_assert_eq!(
                framework.declaration_role(&format!("upgrade_{suffix}")),
                Some("migration_up")
            );
            prop_assert_eq!(
                framework.declaration_role(&format!("downgrade_{suffix}")),
                Some("migration_down")
            );
        }

        #[test]
        fn reported_context_preserves_arbitrary_role_provenance(
            role in "[a-z_]{1,24}",
            framework in "[a-z0-9_]{1,24}",
            implementation_id in "[A-Za-z0-9_./-]{1,32}",
            path_scope in "/[A-Za-z0-9_./*-]{1,40}",
            source in "[a-z_]{1,16}",
            confidence in "[a-z_]{1,16}",
            evidence in prop::collection::vec("[A-Za-z0-9_:./-]{1,40}", 0..8),
            declaration_role in "[a-z_]{1,24}",
        ) {
            let source_role = ResolvedSourceRole {
                role: role.clone(),
                framework: framework.clone(),
                implementation_id: implementation_id.clone(),
                path: "unused-by-classification".into(),
                path_scope: path_scope.clone(),
                source: source.clone(),
                confidence: confidence.clone(),
                contract_version: "test:v1".into(),
                configuration_status: None,
                declaration_span: None,
                evidence: evidence.clone(),
            };

            let context = reported_role(&source_role, &declaration_role, true).unwrap();

            prop_assert_eq!(context.source_role, role);
            prop_assert_eq!(context.declaration_role, declaration_role);
            prop_assert_eq!(context.framework, framework);
            prop_assert_eq!(context.implementation_id, implementation_id);
            prop_assert_eq!(context.path_scope, path_scope);
            prop_assert_eq!(context.source, source);
            prop_assert_eq!(context.confidence, confidence);
            prop_assert_eq!(context.evidence, evidence);
            prop_assert_eq!(context.disposition, "reported_by_role");
        }
    }
}
