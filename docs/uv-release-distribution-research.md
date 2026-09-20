# uv's release and binary-distribution model

Research date: 2026-09-20
uv source revision inspected: [`7b090fba99bc89a6670a23de5368d48d2418a756`](https://github.com/astral-sh/uv/tree/7b090fba99bc89a6670a23de5368d48d2418a756)
Concrete wheel inspected: [`uv 0.12.15`](https://pypi.org/project/uv/0.12.15/#files)
Current release/signing pipeline checked: [`uv 0.12.17`](https://github.com/astral-sh/uv/releases/tag/0.12.17)

## Recommendation for Smells

Use one release build matrix to produce two views of the same Rust CLI:

1. **Primary channel:** standalone GitHub Release archives for macOS ARM64/x64,
   Linux ARM64/x64, and Windows x64, with a checksum and provenance attestation for
   every archive.
2. **Optional Python channel:** platform-specific `py3-none-*` wheels containing the
   compiled executable, published with PyPI Trusted Publishing. This enables
   `uv tool install smells==0.2.0`; it does not turn Smells into a Python program.

The standalone archives should come first. They work for Rust, Python, and TypeScript
repositories without requiring Python. Add wheels when `uv tool install` is a useful
organizational install path. Do not create separate scanner builds for each language
Smells analyzes; distribution targets the host OS and CPU, not the scanned language.

Do not copy uv's very large workflow wholesale. Its useful pattern is a small,
auditable pipeline with a target matrix, smoke tests, checksums, attestations, and a
separately approved publication job.

## What uv currently publishes

### Standalone GitHub artifacts

uv uses Rust target triples in artifact names, with the version in the release tag and
URL rather than the filename. Unix artifacts are `uv-<target>.tar.gz`; Windows
artifacts are `uv-<target>.zip`. Release 0.12.15 includes, among others:

```text
uv-aarch64-apple-darwin.tar.gz
uv-x86_64-apple-darwin.tar.gz
uv-aarch64-unknown-linux-gnu.tar.gz
uv-x86_64-unknown-linux-gnu.tar.gz
uv-aarch64-unknown-linux-musl.tar.gz
uv-x86_64-unknown-linux-musl.tar.gz
uv-x86_64-pc-windows-msvc.zip
```

It also publishes a matching `<archive>.sha256` for every archive, an aggregate
`sha256.sum`, source archive and checksum, shell/PowerShell installers, and a
`dist-manifest.json`. The release page exposes the platform-to-artifact mapping and
checksums directly ([0.12.15 release assets](https://github.com/astral-sh/uv/releases/tag/0.12.15#download-uv-01215)).

The checked-in cargo-dist configuration declares `.tar.gz` for Unix, `.zip` for
Windows, the full target list, shell and PowerShell installers, and both GitHub and
Astral's release mirror as hosts
([`dist-workspace.toml`](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/dist-workspace.toml#L7-L71)).

For Smells, start with exactly these targets:

```text
aarch64-apple-darwin
x86_64-apple-darwin
aarch64-unknown-linux-gnu
x86_64-unknown-linux-gnu
x86_64-pc-windows-msvc
```

Add `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl` if Alpine or a
fully static Linux distribution is in scope. Treat GNU and musl as different Linux
artifacts. uv documents that its GNU builds have minimum glibc versions while its musl
binaries are fully static
([platform policy](https://docs.astral.sh/uv/reference/policies/platforms/#linux-versions)).

### Python wheels containing Rust executables

uv's Python package uses Maturin with `bindings = "bin"`. Its `pyproject.toml` points
Maturin at the Rust crate, uses a small `python/uv` package, and strips the release
binary
([`pyproject.toml`](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/pyproject.toml#L1-L56)).

The wheel names use `py3-none-<platform>`, meaning one compiled wheel works across
supported Python 3 versions but only on the named native platform. Representative
0.12.15 files are:

```text
uv-0.12.15-py3-none-macosx_11_0_arm64.whl
uv-0.12.15-py3-none-macosx_10_12_x86_64.whl
uv-0.12.15-py3-none-manylinux_2_28_aarch64.whl
uv-0.12.15-py3-none-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
uv-0.12.15-py3-none-musllinux_1_1_x86_64.whl
uv-0.12.15-py3-none-win_amd64.whl
uv-0.12.15-py3-none-win_arm64.whl
```

PyPI lists the platform tags and a SHA-256 digest for every file, and records that the
files were uploaded through Trusted Publishing
([uv 0.12.15 files on PyPI](https://pypi.org/project/uv/0.12.15/#files)). uv's platform
policy maps the GNU Rust targets to their `manylinux` tags and the musl targets to
their `musllinux` tags
([platform policy](https://docs.astral.sh/uv/reference/policies/platforms/#linux-versions)).

Inspection of the published macOS ARM64 wheel shows this relevant layout:

```text
uv/__init__.py
uv/__main__.py
uv/_find_uv.py
uv-0.12.15.data/scripts/uv
uv-0.12.15.data/scripts/uvx
uv-0.12.15.dist-info/METADATA
uv-0.12.15.dist-info/sboms/uv.cyclonedx.json
```

The executables are installed as commands from the wheel's `.data/scripts` directory;
Python is the transport and optional `python -m uv` launcher, not the implementation.
The inspected artifact is the published
[`macOS ARM64 wheel`](https://files.pythonhosted.org/packages/84/62/82e86e03e111463ab224c132c0f0e6b649d98d22710b177fbc000d379103/uv-0.12.15-py3-none-macosx_11_0_arm64.whl).
uv also has an exact wheel-content check in its release pipeline, including the
`.data/scripts` executables and embedded SBOM
([wheel-content check](https://github.com/astral-sh/uv/blob/0.12.17/scripts/check_uv_wheel_contents.py)).

uv's build workflow creates wheels with the pinned Maturin action, installs each wheel
from a local directory, runs its commands as smoke tests, and then archives the target
binaries for the standalone channel. It therefore validates both packaging forms in
the platform job
([macOS jobs](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/build-release-binaries.yml#L104-L268),
[Windows jobs](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/build-release-binaries.yml#L269-L440)).

One important difference from Smells' desired guarantee: uv also publishes a source
distribution, and its documentation says installation falls back to building from
source with Rust when no compatible wheel exists
([installation documentation](https://docs.astral.sh/uv/getting-started/installation/#pypi)).
If Smells must never compile during a Python-registry install, either omit the sdist or
require binary-only installation and ensure every supported platform has a wheel.

## Integrity, provenance, and SBOMs

uv uses several distinct mechanisms; they should not be conflated:

- **Checksums:** every standalone archive gets its own `.sha256`, and the release has
  an aggregate `sha256.sum`. The build jobs calculate the digest immediately after
  archiving
  ([macOS example](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/build-release-binaries.yml#L139-L155),
  [Windows example](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/build-release-binaries.yml#L371-L387)).
- **GitHub build provenance:** the publication job applies GitHub Artifact
  Attestations to JSON manifests, installers, ZIP files, and tarballs. The release page
  documents `gh attestation verify ... --repo astral-sh/uv`
  ([release workflow](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/release.yml#L352-L418),
  [verification instructions](https://github.com/astral-sh/uv/releases/tag/0.12.15#verifying-github-artifact-attestations)).
- **PyPI identity:** publication uses OIDC Trusted Publishing with only
  `id-token: write`; there is no long-lived PyPI token in that job
  ([`publish-pypi.yml`](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/publish-pypi.yml#L16-L37)).
- **Native code signing:** uv signs and notarizes the macOS executables, signs Windows
  executables with SHA-256 Authenticode and an RFC 3161 timestamp, verifies the signer,
  and injects the signed programs back into both wheels and GitHub archives
  ([signing workflow](https://github.com/astral-sh/uv/blob/0.12.17/.github/workflows/sign-release-binaries.yml#L141-L298),
  [artifact assembly](https://github.com/astral-sh/uv/blob/0.12.17/.github/workflows/sign-release-binaries.yml#L300-L386)).
- **Embedded executable dependency data:** uv routes release compilation through
  `cargo auditable`; its installer script describes this as SBOM embedding
  ([Cargo wrapper](https://github.com/astral-sh/uv/blob/0.12.17/scripts/cargo.sh),
  [pinned cargo-auditable install](https://github.com/astral-sh/uv/blob/0.12.17/scripts/install-cargo-extensions.sh)).
- **SBOM:** the inspected wheel embeds a CycloneDX 1.5 SBOM under
  `.dist-info/sboms/`, covering the Rust dependency graph. The 0.12.15 standalone
  GitHub asset list does not expose a separately named SBOM file. Smells should improve
  on this by attaching an explicit CycloneDX or SPDX SBOM to the GitHub Release as
  well as embedding it in wheels.

A checksum only detects a mismatch against an expected value. The GitHub attestation
ties the artifact digest to the repository workflow identity. Smells should publish
both, and consumers that need a trust decision should verify the attestation rather
than trust a checksum fetched from the same release page as the binary.

OS-native signing solves a different problem from provenance: it lets Gatekeeper and
Windows verify a recognized publisher when the downloaded executable runs. It is
worth adding before broadly distributing macOS and Windows binaries, but it requires
Apple and Windows signing identities and secure signing infrastructure. It should not
block an initial checksum-and-attestation release if those identities are not yet
available.

## Version and release flow

uv keeps the package version in both Python and Cargo metadata and lists other files
that must be updated by its release-preparation tool
([version metadata](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/pyproject.toml#L3-L8),
[version-file list](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/pyproject.toml#L57-L81)).

Its current release workflow is manually dispatched with a SemVer-like tag. It plans
and builds before publication, uses a dedicated release gate requiring approval, and
finally creates the GitHub Release and tag at the exact workflow commit. Prerelease
suffixes mark the release as a prerelease
([release input and gate](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/release.yml#L17-L100),
[release creation](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/release.yml#L352-L418)).

Recommended Smells flow:

1. Merge a normal PR that changes `Cargo.toml` (and `pyproject.toml` if wheels are
   enabled) to the same exact version and updates the changelog.
2. Require the ordinary CI checks on that PR: formatting, linting, locked build,
   tests, OSV/dependency audit, Smells self-scan, CRAP gate, and packaging dry-run.
3. After the version PR reaches protected `main`, manually dispatch the release for
   tag `v0.2.0` or `0.2.0`; choose one convention and enforce it.
4. Have the workflow reject a tag whose version does not equal the package metadata or
   whose commit is not the current protected release commit.
5. Build, smoke-test, checksum, generate the SBOM, and attest all artifacts before the
   publication job is allowed to run.
6. Publish GitHub assets and, if enabled, all wheels through a protected `release`
   environment using OIDC Trusted Publishing.

The release job should use least-privilege permissions, pinned action commit SHAs,
`persist-credentials: false`, and no repository write token in build jobs. uv applies
those controls throughout its release workflows
([release workflow](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/release.yml),
[binary-build workflow](https://github.com/astral-sh/uv/blob/7b090fba99bc89a6670a23de5368d48d2418a756/.github/workflows/build-release-binaries.yml)).

## Minimal Smells artifact contract

For `v0.2.0`, the first release can be:

```text
smells-aarch64-apple-darwin.tar.gz
smells-aarch64-apple-darwin.tar.gz.sha256
smells-x86_64-apple-darwin.tar.gz
smells-x86_64-apple-darwin.tar.gz.sha256
smells-aarch64-unknown-linux-gnu.tar.gz
smells-aarch64-unknown-linux-gnu.tar.gz.sha256
smells-x86_64-unknown-linux-gnu.tar.gz
smells-x86_64-unknown-linux-gnu.tar.gz.sha256
smells-x86_64-pc-windows-msvc.zip
smells-x86_64-pc-windows-msvc.zip.sha256
sha256.sum
smells.cyclonedx.json
```

Every archive should contain `smells`/`smells.exe`, `LICENSE`, and a short install
README. Every platform job should extract its archive and execute at least
`smells --version` and `smells --help`. If wheels are enabled, add the corresponding
five `smells-0.2.0-py3-none-<platform>.whl` files and smoke-test them with a local,
index-disabled install before publishing.
