SHELL := /bin/sh

.PHONY: build check-tools crap-gate fmt gitleaks install-hooks osv-scan \
	pre-commit pre-commit-push pre-push self-smell-check test

# Both Git hooks deliberately run the same fail-closed gate. This makes a manual
# invocation identical to the checks performed immediately before commit/push.
pre-commit: pre-commit-push

pre-push: pre-commit-push

pre-commit-push: check-tools fmt build test self-smell-check crap-gate gitleaks osv-scan

check-tools:
	@./scripts/check-quality-tools.sh

fmt:
	cargo fmt --all -- --check

build:
	cargo build --locked --all-targets
	cargo clippy --locked --all-targets -- -D warnings

test:
	cargo test --locked

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
