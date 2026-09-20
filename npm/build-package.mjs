#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.dirname(packageRoot);
const platforms = Object.freeze(
  JSON.parse(readFileSync(path.join(packageRoot, "platforms.json"), "utf8")),
);
const [kind, binary, output, version] = process.argv.slice(2);

function npmInvocation() {
  if (process.platform !== "win32") {
    return { command: "npm", prefix: [] };
  }

  const executableDirectory = path.dirname(process.execPath);
  const candidates = [
    path.join(executableDirectory, "node_modules", "npm", "bin", "npm-cli.js"),
    path.join(
      executableDirectory,
      "..",
      "lib",
      "node_modules",
      "npm",
      "bin",
      "npm-cli.js",
    ),
  ];
  const npmCli = candidates.find((candidate) => existsSync(candidate));
  if (!npmCli) {
    throw new Error(
      `cannot locate npm-cli.js beside the Node executable: ${process.execPath}`,
    );
  }
  return { command: process.execPath, prefix: [npmCli] };
}

if (!kind || !binary || !output || !/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("usage: build-package.mjs TARGET|root BINARY|- OUTPUT VERSION");
  process.exit(2);
}
if (kind !== "root" && !platforms[kind]) {
  console.error(`unsupported npm package target: ${kind}`);
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
  const readme = readFileSync(path.join(packageRoot, "README.md"), "utf8");
  writeFileSync(
    path.join(temporary, "README.md"),
    readme.replaceAll("__SMELLS_VERSION__", version),
  );

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
      files: ["bin/smells.js", "bin/platforms.json", "LICENSE", "README.md"],
      engines: { node: ">=18" },
      optionalDependencies,
    };
    mkdirSync(path.join(temporary, "bin"));
    const launcher = path.join(temporary, "bin", "smells.js");
    copyFileSync(path.join(packageRoot, "smells.js"), launcher);
    copyFileSync(
      path.join(packageRoot, "platforms.json"),
      path.join(temporary, "bin", "platforms.json"),
    );
    chmodSync(launcher, 0o755);
  } else {
    const platform = platforms[kind];
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
  const npm = npmInvocation();
  const packed = spawnSync(
    npm.command,
    [...npm.prefix, "pack", temporary, "--pack-destination", output],
    { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] },
  );
  if (packed.error) {
    console.error(`failed to start npm pack: ${packed.error.message}`);
    process.exit(1);
  }
  if (packed.status !== 0) {
    process.exit(packed.status ?? 1);
  }
  process.stdout.write(packed.stdout);
} finally {
  rmSync(temporary, { force: true, recursive: true });
}
