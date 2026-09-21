# Package distribution

Smells has one Rust implementation, four package formats, and five publication
endpoints for that same CLI.
The package channel never changes which Rust, Python, or TypeScript rules execute.
All registry payloads come from the checksummed, immutable
[GitHub Release](https://github.com/mindful-time/smells/releases/tag/v0.4.0).

| Channel | Package | User command | Delivery behavior |
| --- | --- | --- | --- |
| GitHub | release archives | download the matching archive | prebuilt native executable |
| PyPI | `smells` | `uv tool install smells==0.4.0` | prebuilt native wheel; no sdist |
| npm | `@mindful-time/smells` | `npm install --save-dev --save-exact @mindful-time/smells@0.4.0` | exact-version optional native package |
| GitHub Packages | `@mindful-time/smells` | configure the `@mindful-time` scope for `npm.pkg.github.com`, then install the same exact version | authenticated npm mirror linked to this repository |
| crates.io | `smells` | `cargo install --locked --version 0.4.0 smells` | builds from source with Rust 1.88 |

The PyPI design follows uv's binary-wheel model: Python packaging transports the
compiled executable, while `uv tool install` creates an isolated tool environment.
See [uv's tool documentation](https://docs.astral.sh/uv/concepts/tools/) and
[package publishing guide](https://docs.astral.sh/uv/guides/package/).

The npm root package is a small Node launcher. It has five exact-version optional
dependencies and starts the one matching `process.platform` and `process.arch`:

- `@mindful-time/smells-darwin-arm64`
- `@mindful-time/smells-darwin-x64`
- `@mindful-time/smells-linux-arm64-gnu`
- `@mindful-time/smells-linux-x64-gnu`
- `@mindful-time/smells-win32-x64-msvc`

There is no `postinstall` script and no runtime binary download. npm's documented
`os`, `cpu`, and Linux `libc` metadata prevents incompatible native packages from
being selected. Omitting optional dependencies is unsupported and produces a clear
exit-2 error instead of falling back to a network fetch.

The crates.io package is the minimal source closure needed to compile the executable.
`cargo package --locked` rebuilds that closure during CI. Cargo installation is the
only registry path that compiles locally; users who do not want a Rust toolchain use
GitHub, PyPI, or npm.

## Registry ownership bootstrap

Registry identities are global and publication is permanent. The repository contains
no registry credential.

### PyPI

The `smells` project is live on PyPI. Its Trusted Publisher uses these exact values:

| Field | Value |
| --- | --- |
| PyPI project | `smells` |
| GitHub owner | `mindful-time` |
| Repository | `smells` |
| Workflow | `publish-pypi.yml` |
| Environment | `release` |

PyPI can create the project on the first OIDC publication. Trusted Publishing uses a
short-lived token, so no PyPI secret belongs in GitHub. See
[PyPI's Trusted Publisher documentation](https://docs.pypi.org/trusted-publishers/).

### npm

The unscoped npm package `smells` belongs to another project, so Smells uses the
`@mindful-time` scope. The six `v0.3.0` packages were created through the one-time,
interactive owner bootstrap because npm cannot attach a Trusted Publisher until a
package already exists. The retained bootstrap procedure is:

1. Dispatch the owner-gated initial release. Its npmjs.com job is expected to fail
   after the immutable GitHub Release exists because the six packages have no Trusted
   Publisher yet.
2. Run `./scripts/bootstrap-npm-release.sh v0.3.0`. The helper refuses token
   environment variables and creates an isolated temporary npm configuration. It
   starts `npm login --auth-type=web`, requires the authenticated identity to be
   `mindfultime`, downloads every asset from the immutable release, verifies
   `sha256.sum`, publishes the five native packages before the root launcher, and
   verifies each registry SHA-512 digest. Its exit trap logs out and removes the
   temporary configuration, including after an interrupted or failed publication.
3. Configure each package's GitHub Actions Trusted Publisher as owner
   `mindful-time`, repository `smells`, workflow `publish-npm.yml`, environment
   `release`, and allow `npm publish`.
4. Require two-factor authentication and disallow traditional token publishing for
   every package.
5. Dispatch **Publish npm from release** for the existing tag. The deterministic
   publisher sees the exact release digests and skips packages already present.

No npm publishing secret belongs in GitHub. The manual `v0.3.0` bootstrap cannot
produce npm's CI-bound provenance, but its tarballs must be byte-identical to the
checksummed immutable GitHub Release. Subsequent releases use npm's short-lived OIDC
identity and automatic registry provenance.
The workflow requires npm 11.5.1 or newer, as documented by
[npm Trusted Publishing](https://docs.npmjs.com/trusted-publishers/).

### GitHub Packages

The same six npm tarballs are mirrored to `npm.pkg.github.com` after the immutable
GitHub Release succeeds. Both the normal release and the manually dispatchable
`Publish GitHub Packages from release` recovery workflow call the same publisher.
It verifies the release attestation, downloads the exact `.tgz` assets, compares
registry SHA-512 digests, and only publishes missing versions. The workflow grants
only `packages: write` and authenticates with its short-lived `GITHUB_TOKEN`; no
additional repository secret is stored.

Every package manifest links to `mindful-time/smells`, so GitHub associates the six
packages with this repository and they appear on its Packages page. Repository access
permissions are inherited, but package visibility is separate: GitHub creates new npm
packages as private, so the owner must change each package to public after its first
publication. GitHub's npm registry requires authenticated installs even for public
packages; the ordinary unauthenticated installation path remains npmjs.com. See
[GitHub's npm package documentation](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-npm-registry).

### crates.io

crates.io requires the first package release to be published manually. After that
bootstrap, configure the crate's Trusted Publisher with owner `mindful-time`,
repository `smells`, workflow `publish-crates.yml`, and environment `release`. The
publisher workflow uses `rust-lang/crates-io-auth-action` to exchange the protected
GitHub OIDC identity for a short-lived token and revokes it when the job finishes; no
long-lived `CARGO_REGISTRY_TOKEN` secret belongs in GitHub. Cargo documents the
permanent version semantics and required pre-publication verification in
[Publishing on crates.io](https://doc.rust-lang.org/cargo/reference/publishing.html),
and crates.io documents the one-time first-release requirement in
[Trusted Publishing](https://crates.io/docs/trusted-publishing).

## Release guarantees

The owner-dispatched release first passes the complete quality gate and natively
builds all five targets. Every platform job then installs and runs its wheel,
standalone archive, and npm package without an index or install script. The gated
publication job creates one immutable GitHub Release containing:

- five standalone archives and their checksums;
- five Python wheels;
- the five native npm packages and root npm launcher;
- the crates.io source archive;
- aggregate checksums and CycloneDX/SPDX SBOMs.

Only after the signed GitHub Release succeeds do independent reusable workflows
publish to PyPI, npmjs.com, GitHub Packages, and crates.io. Every publisher uses the
protected `release` environment, rechecks the release actor and immutable release,
and installs the exact version back from its registry. The same workflows can be
dispatched independently to recover a partial registry failure without rebuilding or
mutating release assets. The crates.io job also requires its newly built archive to
be byte-identical to the attested `.crate` asset and uses that asset's digest for
registry verification.
