# Changelog

All notable changes to Smells are recorded here. Release tags use `vMAJOR.MINOR.PATCH`.

## 0.4.0 - 2026-09-21

- Run all 28 rules and report all 23 canonical smells by default without requiring
  consuming repositories to generate provider-evidence bundles.
- Add built-in Rust, Python, and TypeScript collectors for type-shape, CRAP risk,
  dead code, architecture, inheritance, delegation, and coupling indicators.
- Capture up to 200 Git commits automatically for divergent-change and
  shotgun-surgery indicators, binding the captured history into `input_sha256`;
  selected history rules are explicitly incomplete when Git is unavailable.
- Keep `--evidence` as an optional per-rule higher-fidelity override.
- Report only scanner-owned inputs as required; compiler, coverage, type, test,
  and project-contract bundles are optional overrides.
- Preserve non-negotiable Refactoring.Guru research: an agent must open and read
  the exact emitted URL before review or remediation, or stop as incomplete.

## 0.3.0 - 2026-09-20

- Prepare the prebuilt scanner wheels as the `smells` PyPI tool package.
- Add `@mindful-time/smells` plus five exact-version native npm packages without
  install scripts or runtime downloads, published identically to npmjs.com and
  GitHub Packages.
- Prepare the verified Rust source package as `smells` on crates.io.
- Bootstrap the first npm release interactively from the immutable GitHub Release;
  subsequent npm releases use tokenless Trusted Publishing.
- Verify every registry by installing the released version after publication.

## 0.2.1 - 2026-09-20

- License the project under the OSI-approved MIT License.
- Embed `LICENSE` in standalone archives and binary Python wheels.

## 0.2.0 - 2026-09-20

- Activate all 28 policy rules by default for Rust, Python, and TypeScript.
- Add deterministic uv-style policy group selection and resolved-policy reporting.
- Add prebuilt standalone archives and binary Python wheels for five host targets.
- Add fork-only pull-request CI, release checksums, CycloneDX/SPDX SBOMs, and
  signed immutable GitHub Release attestations.

## 0.1.0 - 2026-09-19

- Initial deterministic multi-language scanner and self-hosted quality gates.
