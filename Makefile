SHELL := /bin/sh

.PHONY: build check-tools crap-gate fmt gitleaks install-hooks osv-scan \
	pre-commit pre-commit-push pre-push release-check self-smell-check test

# Both Git hooks deliberately run the same fail-closed gate. This makes a manual
# invocation identical to the checks performed immediately before commit/push.
pre-commit: pre-commit-push

pre-push: pre-commit-push

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
		scripts/package-npm.sh scripts/smoke-test-npm.sh \
		scripts/publish-crate.sh \
		scripts/configure-main-protection.sh scripts/verify-release-actor.sh \
		scripts/collect-release-artifacts.sh tests/release-artifacts.sh \
		tests/npm-packages.sh
	@sh tests/release-artifacts.sh
	@sh tests/npm-packages.sh
	@node --check npm/smells.js
	@node --check npm/build-package.mjs
	@node --check npm/publish-packages.mjs
	@PYTHONPYCACHEPREFIX="$${TMPDIR:-/tmp}/smells-pycache" \
		python3 -m py_compile scripts/verify-pypi-release.py
	@cargo package --locked --allow-dirty --list >/dev/null
	@version=$$(sed -n '/^\[package\]/,/^\[/ s/^version = "\([^"]*\)"/\1/p' Cargo.toml); \
		./scripts/verify-release-tag.sh "v$$version" >/dev/null
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
