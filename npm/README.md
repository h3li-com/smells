# `@mindful-time/smells`

This package installs the native `smells` executable for the current supported host.
The JavaScript entry point only selects and starts the exact-version platform package;
the deterministic scanner itself remains the Rust binary from
[`mindful-time/smells`](https://github.com/mindful-time/smells).

```sh
npm install --save-dev --save-exact @mindful-time/smells@__SMELLS_VERSION__
npx --no-install smells --version
```

Optional dependencies must not be omitted. Supported hosts are macOS ARM64/x64,
Linux glibc ARM64/x64, and Windows x64.
