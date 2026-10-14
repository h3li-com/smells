SHELL := /bin/sh
mutation_shard_arg = $(if $(strip $(MUTATION_SHARD)),--shard $(MUTATION_SHARD) --sharding round-robin,)

.PHONY: benchmark-release-gate build check-tools crap-gate fmt gitleaks install-hooks \
	mutation-diff mutation-full mutation-test-core osv-scan pre-commit pre-commit-push \
	pre-push real-repository-e2e release-check self-smell-check test

# Both Git hooks deliberately run the same fail-closed gate. This makes a manual
# invocation identical to the checks performed immediately before commit/push.
pre-commit: pre-commit-push

pre-push: pre-commit-push

benchmark-release-gate:
	cargo test --locked --test multilanguage alternative_interface_budget_charges_only_surviving_exact_comparisons -- --exact
	@./scripts/benchmark-release-gate.sh

mutation-test-core:
	mkdir -p target/mutants
	@./scripts/run-mutation-test.sh target/mutants/core-migration \
		--file src/core/migration.rs -- --lib core::migration::tests

mutation-diff:
	@test -n "$(MUTATION_DIFF)" || \
		(echo 'MUTATION_DIFF must name a Git patch file' >&2; exit 2)
	mkdir -p target/mutants
	@MUTATION_IN_PLACE=false \
		CARGO_MUTANTS_JOBS="$${CARGO_MUTANTS_JOBS:-4}" \
		./scripts/run-mutation-test.sh target/mutants/diff \
		--copy-vcs true $(mutation_shard_arg) --in-diff "$(MUTATION_DIFF)"

mutation-full:
	mkdir -p target/mutants
	@MUTATION_IN_PLACE=false \
		CARGO_MUTANTS_JOBS="$${CARGO_MUTANTS_JOBS:-4}" \
		./scripts/run-mutation-test.sh target/mutants/full \
		--copy-vcs true $(mutation_shard_arg)

real-repository-e2e:
	cargo build --locked --release
	SMELLS_BIN="$(CURDIR)/target/release/smells" ./tests/real-repositories.sh

pre-commit-push: check-tools fmt build test release-check self-smell-check crap-gate gitleaks osv-scan

check-tools:
	@./scripts/check-quality-tools.sh

fmt:
	cargo fmt --all -- --check

build:
	cargo build --locked --all-targets
	cargo clippy --locked --all-targets -- -D warnings

test:
	cargo test --locked

release-check:
	@sh -n scripts/quality-tool-versions.sh scripts/install-ci-quality-tools.sh \
		scripts/verify-release-tag.sh scripts/package-release.sh \
		scripts/verify-release-notes.sh \
		scripts/package-npm.sh scripts/smoke-test-npm.sh \
		scripts/publish-crate.sh scripts/bootstrap-npm-release.sh \
		scripts/configure-main-protection.sh scripts/verify-release-actor.sh \
		scripts/verify-registry-release.sh scripts/dispatch-registry-publications.sh \
		scripts/benchmark-release-gate.sh scripts/prepare-mutation-diff.sh \
		scripts/run-mutation-test.sh \
		scripts/collect-release-artifacts.sh tests/release-artifacts.sh \
		tests/release-notes.sh \
		tests/mutation-runner.sh tests/mutation-workflow.sh \
		tests/real-repositories.sh \
		tests/npm-packages.sh tests/npm-publisher.sh tests/npm-bootstrap.sh \
		tests/crate-package.sh tests/registry-publication.sh \
		tests/release-workflow.sh
	@sh tests/release-artifacts.sh
	@sh tests/npm-packages.sh
	@sh tests/npm-publisher.sh
	@sh tests/npm-bootstrap.sh
	@sh tests/crate-package.sh
	@sh tests/registry-publication.sh
	@sh tests/release-workflow.sh
	@sh tests/release-notes.sh
	@sh tests/mutation-runner.sh
	@sh tests/mutation-workflow.sh
	@node --check npm/smells.js
	@node --check npm/build-package.mjs
	@node --check npm/publish-packages.mjs
	@PYTHONPYCACHEPREFIX="$${TMPDIR:-/tmp}/smells-pycache" \
		python3 -m py_compile \
			scripts/mutation-summary.py scripts/verify-pypi-release.py
	@version=$$(sed -n '/^\[package\]/,/^\[/ s/^version = "\([^"]*\)"/\1/p' Cargo.toml); \
		./scripts/verify-release-tag.sh "v$$version" >/dev/null; \
		./scripts/verify-release-notes.sh "v$$version" >/dev/null
	@./scripts/verify-release-actor.sh mindful-time mindful-time mindful-time >/dev/null
	@if ./scripts/verify-release-actor.sh \
		mindful-time mindful-time another-user >/dev/null 2>&1; then \
		echo 'release actor guard accepted a non-owner' >&2; \
		exit 1; \
	fi

self-smell-check: build
	@./scripts/self-smell-check.sh

crap-gate: check-tools
	@./scripts/crap-gate.sh

gitleaks: check-tools
	gitleaks git --staged --redact=100 --no-banner .
	gitleaks git --redact=100 --no-banner .

osv-scan: check-tools
	osv-scanner scan source --lockfile Cargo.lock

install-hooks:
	@./scripts/install-hooks.sh
