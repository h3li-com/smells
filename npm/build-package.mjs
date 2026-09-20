#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const platforms = Object.freeze({
  "aarch64-apple-darwin": {
    name: "@mindful-time/smells-darwin-arm64",
    os: ["darwin"],
    cpu: ["arm64"],
    executable: "smells",
  },
  "x86_64-apple-darwin": {
    name: "@mindful-time/smells-darwin-x64",
    os: ["darwin"],
    cpu: ["x64"],
    executable: "smells",
  },
  "aarch64-unknown-linux-gnu": {
    name: "@mindful-time/smells-linux-arm64-gnu",
    os: ["linux"],
    cpu: ["arm64"],
    libc: ["glibc"],
    executable: "smells",
  },
  "x86_64-unknown-linux-gnu": {
    name: "@mindful-time/smells-linux-x64-gnu",
    os: ["linux"],
    cpu: ["x64"],
    libc: ["glibc"],
    executable: "smells",
  },
  "x86_64-pc-windows-msvc": {
    name: "@mindful-time/smells-win32-x64-msvc",
    os: ["win32"],
    cpu: ["x64"],
    executable: "smells.exe",
  },
});

const packageRoot = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.dirname(packageRoot);
const [kind, binary, output, version] = process.argv.slice(2);

if (!kind || !binary || !output || !/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("usage: build-package.mjs TARGET|root BINARY|- OUTPUT VERSION");
  process.exit(2);
}

const common = {
  version,
  description:
    "Deterministic Rust, Python, and TypeScript source-pattern smell scanner",
  license: "MIT",
  author: "mindful-time",
  homepage: "https://github.com/mindful-time/smells#readme",
  repository: {
    type: "git",
    url: "git+https://github.com/mindful-time/smells.git",
  },
  bugs: { url: "https://github.com/mindful-time/smells/issues" },
  keywords: ["code-quality", "code-smells", "rust", "python", "typescript"],
  publishConfig: { access: "public" },
};

mkdirSync(output, { recursive: true });
const temporary = mkdtempSync(path.join(tmpdir(), "smells-npm-package."));

try {
  copyFileSync(path.join(repositoryRoot, "LICENSE"), path.join(temporary, "LICENSE"));
  copyFileSync(path.join(packageRoot, "README.md"), path.join(temporary, "README.md"));

  let manifest;
  if (kind === "root") {
    const optionalDependencies = Object.fromEntries(
      Object.values(platforms).map(({ name }) => [name, version]),
    );
    manifest = {
      ...common,
      name: "@mindful-time/smells",
      type: "commonjs",
      bin: { smells: "bin/smells.js" },
      files: ["bin/smells.js", "LICENSE", "README.md"],
      engines: { node: ">=18" },
      optionalDependencies,
    };
    mkdirSync(path.join(temporary, "bin"));
    const launcher = path.join(temporary, "bin", "smells.js");
    copyFileSync(path.join(packageRoot, "smells.js"), launcher);
    chmodSync(launcher, 0o755);
  } else {
    const platform = platforms[kind];
    if (!platform) {
      console.error(`unsupported npm package target: ${kind}`);
      process.exit(2);
    }
    manifest = {
      ...common,
      name: platform.name,
      files: [`bin/${platform.executable}`, "LICENSE", "README.md"],
      os: platform.os,
      cpu: platform.cpu,
      ...(platform.libc ? { libc: platform.libc } : {}),
    };
    mkdirSync(path.join(temporary, "bin"));
    const executable = path.join(temporary, "bin", platform.executable);
    cpSync(binary, executable);
    chmodSync(executable, 0o755);
  }

  writeFileSync(
    path.join(temporary, "package.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  const packed = spawnSync(
    "npm",
    ["pack", temporary, "--pack-destination", output],
    { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] },
  );
  if (packed.status !== 0) {
    process.exit(packed.status ?? 1);
  }
  process.stdout.write(packed.stdout);
} finally {
  rmSync(temporary, { force: true, recursive: true });
}
