# Release process

Smells has one Rust executable and two distribution views of that executable:

- Standalone GitHub Release archives are the primary, language-neutral channel.
- Maturin `bindings = "bin"` wheels allow installation through Python tooling.

There is no source distribution. A wheel installation must either find a compatible
prebuilt wheel or fail; it must never fall back to compiling Rust locally. Rust,
Python, and TypeScript are scanner rule packs, not separate executable packages.

## Supported release targets

| Host | Rust target | Archive |
| --- | --- | --- |
| macOS Apple Silicon | `aarch64-apple-darwin` | `.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `.tar.gz` |
| Linux ARM64, glibc | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| Linux AMD64, glibc | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Windows AMD64 | `x86_64-pc-windows-msvc` | `.zip` |

Alpine and other musl systems are not covered by the GNU Linux artifacts. Add
separate `*-unknown-linux-musl` builds when that platform enters scope.

Every platform job builds both the wheel and standalone executable from the same
commit. Architecture-matched GitHub-hosted runners install every wheel without
using an index and execute `smells --version` and `smells --help`; ARM64 is tested
on Linux ARM64 and Apple Silicon rather than merely cross-compiled.

Pull-request CI performs the same license-metadata, license-content, wheel, and
archive checks on the representative Linux AMD64 target. The owner-approved release
matrix is the authoritative cross-platform check and repeats them on all five targets.

## Prepare a version

Version changes are normal pull requests. A version PR must update:

1. `Cargo.toml` and `Cargo.lock`;
2. every starter policy and policy schema scanner pin;
3. versioned test evidence and documentation;
4. `CHANGELOG.md`.

`pyproject.toml` reads the version dynamically from Cargo metadata, so it cannot
drift independently. The PR must pass the required `CI / required` check.

## Publish a release

After the version PR is merged into `main`:

1. The repository owner, `mindful-time`, opens **Actions → Release → Run workflow**.
2. Select `main` and enter the exact stable tag, such as `v0.2.1`.
3. The workflow rejects every other dispatch or rerun actor, input other than
   `vMAJOR.MINOR.PATCH`, a tag that already exists, a non-`main` dispatch, or a
   version that differs from `Cargo.toml`. The actor check runs during validation
   and again immediately before publication so a partial job rerun cannot bypass it.
4. The complete local quality gate runs again before any platform build.
5. All five archives and wheels are built and executed on matching architectures.
   The publication job flattens the nested workflow artifacts into a clean staging
   directory and rejects duplicate filenames before it reads or publishes them.
   The publication job produces individual archive checksums, `sha256.sum`, a
   Cargo-dependency `smells.cyclonedx.json`, and an artifact inventory in
   `smells-artifacts.spdx.json`.
6. Only after every step succeeds does the workflow create the tag and immutable
   GitHub Release at the exact tested commit. GitHub then produces a cryptographically
   signed release attestation binding the tag, commit, and asset digests.

The `release` GitHub environment is the publication boundary. Configure that
environment to require the repository owner when the account plan supports required
reviewers. PyPI publication is intentionally absent until a registry, package
ownership, and Trusted Publisher are explicitly configured.

macOS notarization and Windows Authenticode publisher signing are intentionally
pending because they require external identities. Checksums and GitHub's signed
release attestation are present from the first release, but they do not replace
operating-system publisher signing.

## Verify an artifact

Download the archive, its matching `.sha256`, and verify it before extraction:

```sh
sha256sum --check smells-x86_64-unknown-linux-gnu.tar.gz.sha256
gh release verify v0.2.1 --repo mindful-time/smells
gh release verify-asset v0.2.1 \
  smells-x86_64-unknown-linux-gnu.tar.gz \
  --repo mindful-time/smells
```

On macOS, use `shasum -a 256 -c` for the checksum file. The aggregate
`sha256.sum` also covers every archive, wheel, and both release SBOMs.

The design is derived from uv's first-party release implementation; see
[the cited research](uv-release-distribution-research.md).
