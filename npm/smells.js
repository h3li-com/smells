#!/usr/bin/env node
"use strict";

const path = require("node:path");
const { spawnSync } = require("node:child_process");

const packages = Object.freeze({
  "darwin-arm64": ["@mindful-time/smells-darwin-arm64", "smells"],
  "darwin-x64": ["@mindful-time/smells-darwin-x64", "smells"],
  "linux-arm64": ["@mindful-time/smells-linux-arm64-gnu", "smells"],
  "linux-x64": ["@mindful-time/smells-linux-x64-gnu", "smells"],
  "win32-x64": ["@mindful-time/smells-win32-x64-msvc", "smells.exe"],
});

const key = `${process.platform}-${process.arch}`;
const selected = packages[key];

if (!selected) {
  console.error(`smells: unsupported npm host ${key}`);
  process.exit(2);
}

const [packageName, executableName] = selected;
let packageJson;
try {
  packageJson = require.resolve(`${packageName}/package.json`);
} catch (error) {
  if (error && error.code !== "MODULE_NOT_FOUND") {
    throw error;
  }
  console.error(
    `smells: native package ${packageName} is unavailable; reinstall @mindful-time/smells without omitting optional dependencies`,
  );
  process.exit(2);
}

const executable = path.join(path.dirname(packageJson), "bin", executableName);
const result = spawnSync(executable, process.argv.slice(2), {
  stdio: "inherit",
  windowsHide: true,
});

if (result.error) {
  console.error(`smells: failed to start ${executable}: ${result.error.message}`);
  process.exit(2);
}
if (result.signal) {
  console.error(`smells: native executable terminated by ${result.signal}`);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
