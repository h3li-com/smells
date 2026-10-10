#!/bin/sh
set -eu

# This test only reads repository source. It never installs dependencies, imports
# application code, connects to a database, or invokes a migration framework.

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
scanner=${SMELLS_BIN:-$repository_root/target/release/smells}
report_root=${SMELLS_REAL_REPOSITORY_REPORT_DIR:-$repository_root/target/real-repositories}
fixture_root=$(mktemp -d "${TMPDIR:-/tmp}/smells-real-repositories.XXXXXX")

cleanup() {
	rm -rf "$fixture_root"
}
trap cleanup EXIT HUP INT TERM

fail() {
	printf 'real-repository E2E failed: %s\n' "$*" >&2
	exit 1
}

for command in git jq; do
	command -v "$command" >/dev/null 2>&1 || fail "missing required command: $command"
done
test -x "$scanner" || fail "scanner is not executable: $scanner"
mkdir -p "$report_root"

checkout_fixture() {
	name=$1
	url=$2
	revision=$3
	shift 3
	destination=$fixture_root/$name

	git init -q "$destination"
	git -C "$destination" remote add origin "$url"
	git -C "$destination" sparse-checkout init --no-cone
	git -C "$destination" sparse-checkout set "$@"
	git -C "$destination" fetch -q --filter=blob:none --depth=1 origin "$revision"
	git -C "$destination" checkout -q --detach "$revision"

	actual=$(git -C "$destination" rev-parse HEAD)
	test "$actual" = "$revision" || fail "$name resolved to $actual instead of $revision"
	printf 'checked out %s at %s\n' "$name" "$revision"
}

scan_report() {
	source_root=$1
	policy=$2
	report=$3

	set +e
	SMELLS_CACHE_DIR=$fixture_root/cache "$scanner" check \
		--path "$source_root" \
		--policy "$policy" \
		--all-groups \
		--format table \
		--report "$report" \
		>/dev/null
	status=$?
	set -e

	case $status in
	0 | 1) ;;
	*) fail "scanner returned invalid/incomplete exit $status for $source_root" ;;
	esac
}

assert_complete_catalog() {
	report=$1
	jq -e '
        (.errors == []) and
        (.smell_results | length == 23) and
        all(.smell_results[]; .state != "excluded")
    ' "$report" >/dev/null || fail "all smell categories were not evaluated in $report"
}

checkout_fixture \
	fastapi-template \
	https://github.com/fastapi/full-stack-fastapi-template.git \
	7257e606cb94eada1dca40173e2b7f99dd549bc6 \
	'/backend/'
checkout_fixture \
	sea-orm \
	https://github.com/SeaQL/sea-orm.git \
	22f6400e81864aec5438d0447a4c4d9b51df6527 \
	'/examples/actix_example/migration/'
checkout_fixture \
	typeorm \
	https://github.com/typeorm/typeorm.git \
	5a8a1400361a4b5cb90e805bea2c665149da3911 \
	'/packages/typeorm/package.json' \
	'/packages/typeorm/test/github-issues/2693/migration/'
checkout_fixture \
	drizzle-application \
	https://github.com/syntaxfm/live-tools.git \
	6cd7a294f95909b59bd51f19913a00ff3f950fa8 \
	'/package.json' \
	'/drizzle.config.ts' \
	'/drizzle/migrations/'

python_policy=$repository_root/examples/python-quality-policy.json
rust_policy=$repository_root/examples/quality-policy.json
typescript_policy=$repository_root/examples/typescript-quality-policy.json

fastapi_report=$report_root/fastapi-alembic.json
scan_report "$fixture_root/fastapi-template/backend" "$python_policy" "$fastapi_report"
assert_complete_catalog "$fastapi_report"
jq -e '
    any(.source_roles[];
        .framework == "alembic" and
        .source == "inferred" and
        .confidence == "high") and
    any(.measurements[];
        .role_context != null and
        .role_context.framework == "alembic" and
        .role_context.declaration_role == "migration_up") and
    any(.measurements[];
        .role_context != null and
        .role_context.framework == "alembic" and
        .role_context.declaration_role == "migration_down")
' "$fastapi_report" >/dev/null || fail 'FastAPI Alembic revisions were not classified'

sea_orm_report=$report_root/sea-orm.json
scan_report \
	"$fixture_root/sea-orm/examples/actix_example/migration" \
	"$rust_policy" \
	"$sea_orm_report"
assert_complete_catalog "$sea_orm_report"
jq -e '
    ([.source_roles[] |
        select(.framework == "sea_orm" and .source == "inferred" and .confidence == "high")]
        | length) >= 2 and
    any(.measurements[];
        .role_context != null and
        .role_context.framework == "sea_orm" and
        .role_context.declaration_role == "migration_shell")
' "$sea_orm_report" >/dev/null || fail 'SeaORM migrations were not classified'

typeorm_unresolved_report=$report_root/typeorm-unresolved.json
scan_report \
	"$fixture_root/typeorm/packages/typeorm" \
	"$typescript_policy" \
	"$typeorm_unresolved_report"
assert_complete_catalog "$typeorm_unresolved_report"
jq -e '
    (.source_roles == []) and
    ([.source_role_candidates[] |
        select(.evidence == ["conventional_path_component:migration"])] | length) >= 3
' "$typeorm_unresolved_report" >/dev/null || \
	fail 'TypeORM path-only candidates did not remain fail-closed'

typeorm_policy=$fixture_root/typeorm-policy.json
jq '
    .source_roles = [{
        "role": "database_migration",
        "framework": "typeorm",
        "paths": ["/test/github-issues/2693/migration/*.ts"]
    }]
' "$typescript_policy" >"$typeorm_policy"

typeorm_report=$report_root/typeorm-declared.json
scan_report \
	"$fixture_root/typeorm/packages/typeorm" \
	"$typeorm_policy" \
	"$typeorm_report"
assert_complete_catalog "$typeorm_report"
jq -e '
    ([.source_roles[] |
        select(.framework == "typeorm" and .source == "declared" and .confidence == "explicit")]
        | length) >= 3 and
    ([.measurements[] |
        select(.role_context != null and .role_context.framework == "typeorm") |
        .role_context.declaration_role] | unique | sort) ==
        ["migration_down", "migration_shell", "migration_up"]
' "$typeorm_report" >/dev/null || fail 'declared TypeORM roles did not bind canonical methods'

drizzle_report=$report_root/drizzle.json
scan_report \
	"$fixture_root/drizzle-application" \
	"$typescript_policy" \
	"$drizzle_report"
assert_complete_catalog "$drizzle_report"
jq -e '
    any(.migration_artifacts[];
        .framework == "drizzle_kit" and
        .path == "drizzle/migrations/0000_wealthy_sway.sql" and
        .status == "migration_body_not_analyzed")
' "$drizzle_report" >/dev/null || fail 'Drizzle SQL migration was not inventoried'

printf 'real-repository E2E passed; reports: %s\n' "$report_root"
