#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const [directory, version, registry, publicationMode, ...extraArguments] =
  process.argv.slice(2);
const registries = new Set([
  "https://registry.npmjs.org",
  "https://npm.pkg.github.com",
]);
if (
  !directory ||
  !/^\d+\.\d+\.\d+$/.test(version ?? "") ||
  !registries.has(registry) ||
  ![undefined, "--provenance"].includes(publicationMode) ||
  extraArguments.length > 0
) {
  console.error(
    "usage: publish-packages.mjs DIRECTORY VERSION REGISTRY [--provenance]",
  );
  process.exit(2);
}
if (publicationMode === "--provenance" && registry !== "https://registry.npmjs.org") {
  console.error("npm provenance is only enabled for registry.npmjs.org");
  process.exit(2);
}

const packageRoot = path.dirname(fileURLToPath(import.meta.url));
const platforms = JSON.parse(
  readFileSync(path.join(packageRoot, "platforms.json"), "utf8"),
);
const packages = [
  ...Object.values(platforms).map(({ name }) => [
    name,
    `${name.slice(1).replace("/", "-")}-${version}.tgz`,
  ]),
  ["@mindful-time/smells", `mindful-time-smells-${version}.tgz`],
];

const expectedFiles = packages.map(([, filename]) => filename).sort();
const actualFiles = readdirSync(directory)
  .filter((filename) => filename.endsWith(".tgz"))
  .sort();
if (JSON.stringify(actualFiles) !== JSON.stringify(expectedFiles)) {
  console.error(`npm package set mismatch: expected ${expectedFiles.join(", ")}; found ${actualFiles.join(", ")}`);
  process.exit(2);
}

function localIntegrity(filename) {
  const digest = createHash("sha512")
    .update(readFileSync(path.join(directory, filename)))
    .digest("base64");
  return `sha512-${digest}`;
}

function registryIntegrity(name) {
  const result = spawnSync(
    "npm",
    [
      "view",
      `${name}@${version}`,
      "dist.integrity",
      "--json",
      "--registry",
      registry,
    ],
    { encoding: "utf8" },
  );
  if (result.status === 0) {
    const integrity = JSON.parse(result.stdout);
    if (typeof integrity !== "string" || !integrity.startsWith("sha512-")) {
      console.error(`npm returned invalid integrity metadata for ${name}@${version}`);
      process.exit(2);
    }
    return integrity;
  }
  const missing = `${result.stdout}\n${result.stderr}`;
  if (missing.includes("E404") || missing.includes("404 Not Found")) {
    return null;
  }
  process.stderr.write(result.stderr);
  process.exit(result.status ?? 1);
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

for (const [name, filename] of packages) {
  const expected = localIntegrity(filename);
  const existing = registryIntegrity(name);
  if (existing !== null) {
    if (existing !== expected) {
      console.error(`npm digest mismatch for existing ${name}@${version}`);
      process.exit(2);
    }
    console.log(`npm package already matches: ${name}@${version}`);
    continue;
  }

  const publishArguments = [
    "publish",
    path.join(directory, filename),
    "--registry",
    registry,
  ];
  if (registry === "https://registry.npmjs.org") {
    publishArguments.push("--access", "public");
  }
  if (publicationMode === "--provenance") {
    publishArguments.push("--provenance");
  }
  const published = spawnSync("npm", publishArguments, { stdio: "inherit" });
  if (published.status !== 0) {
    const recovered = registryIntegrity(name);
    if (recovered === expected) {
      console.log(`npm publish returned an error after ${name}@${version} became available`);
      continue;
    }
    process.exit(published.status ?? 1);
  }

  let verified = false;
  for (let attempt = 0; attempt < 10; attempt += 1) {
    const remote = registryIntegrity(name);
    if (remote === expected) {
      verified = true;
      break;
    }
    if (remote !== null) {
      console.error(`npm digest mismatch after publishing ${name}@${version}`);
      process.exit(2);
    }
    await sleep(3000);
  }
  if (!verified) {
    console.error(`npm package did not become visible: ${name}@${version}`);
    process.exit(1);
  }
}
