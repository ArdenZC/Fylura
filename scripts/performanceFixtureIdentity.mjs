import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { PERFORMANCE_ARTIFACT_FORMAT_VERSION } from "./performanceArtifactManifest.mjs";
import { resolvePerformanceProfile } from "./performanceProfile.mjs";
import { getFixtureWorkingFiles } from "./performanceManifest.mjs";

const root = process.cwd();
export const PERFORMANCE_FIXTURE_SCHEMA_VERSION = 42;
export const PERFORMANCE_FIXTURE_FORMAT_VERSION = 1;

function parseFlag(argv, name) {
  const index = argv.findIndex((argument) => argument === name || argument.startsWith(`${name}=`));
  if (index < 0) return undefined;
  const argument = argv[index];
  if (argument.startsWith(`${name}=`)) return argument.slice(name.length + 1);
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value.`);
  return value;
}

function hashFile(relativePath) {
  const filePath = path.join(root, relativePath);
  if (!fs.existsSync(filePath)) throw new Error(`Fixture identity input is missing: ${relativePath}`);
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function filesUnder(relativePath) {
  const directory = path.join(root, relativePath);
  if (!fs.existsSync(directory)) return [];
  const result = [];
  const visit = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const resolved = path.join(current, entry.name);
      if (entry.isDirectory()) visit(resolved);
      else result.push(path.relative(root, resolved).replaceAll("\\", "/"));
    }
  };
  visit(directory);
  return result.sort((left, right) => left.localeCompare(right));
}

export function createPerformanceFixtureIdentity({
  profile,
  runnerOs = process.env.RUNNER_OS ?? process.platform,
  runnerArch = process.env.RUNNER_ARCH ?? process.arch,
} = {}) {
  const normalizedProfile = resolvePerformanceProfile([`--profile=${profile ?? "full"}`]);
  const rowCounts = normalizedProfile === "full" ? [100_000, 1_000_000] : [100_000];
  const migrationFiles = filesUnder("src-tauri/src/db/migrations");
  const inputPaths = [
    "scripts/preparePerformanceFixtures.mjs",
    "scripts/performanceFixtureIdentity.mjs",
    "src-tauri/tests/performance_fixture_builder.rs",
    "src-tauri/tests/support/performance_fixture.rs",
    "src-tauri/tests/file_library_performance.rs",
    "src-tauri/src/db/schema.rs",
    "src-tauri/src/db/schema_global_generations.rs",
    "src-tauri/src/db/schema_global_generations.sql",
    "src-tauri/src/db/queries/files.rs",
    ...filesUnder("src-tauri/src/db/queries/library"),
    ...migrationFiles,
  ];
  const inputs = [...new Set(inputPaths)].sort((left, right) => left.localeCompare(right)).map((relativePath) => ({
    path: relativePath,
    sha256: hashFile(relativePath),
  }));
  const payload = {
    identityVersion: 1,
    fixtureType: "file-library-sqlite-working-copies",
    runner: { os: runnerOs, arch: runnerArch },
    schemaVersion: PERFORMANCE_FIXTURE_SCHEMA_VERSION,
    fixtureFormatVersion: PERFORMANCE_FIXTURE_FORMAT_VERSION,
    rowCounts,
    artifactFormatVersion: PERFORMANCE_ARTIFACT_FORMAT_VERSION,
    inputs,
  };
  const fixtureIdentity = crypto
    .createHash("sha256")
    .update(JSON.stringify(payload))
    .digest("hex");
  const fixtureCacheKey = [
    "zen-canvas-perf-fixture",
    runnerOs,
    runnerArch,
    normalizedProfile,
    fixtureIdentity,
  ].join("-");
  return {
    fixtureIdentity,
    fixtureCacheKey,
    fixtureFormatVersion: PERFORMANCE_FIXTURE_FORMAT_VERSION,
    schemaVersion: PERFORMANCE_FIXTURE_SCHEMA_VERSION,
    rowCounts,
    runner: payload.runner,
    payload,
    requiredFiles: getFixtureWorkingFiles("library-content", normalizedProfile),
  };
}

function writeOutput(values) {
  if (!process.env.GITHUB_OUTPUT) return;
  fs.appendFileSync(
    process.env.GITHUB_OUTPUT,
    `${Object.entries(values).map(([key, value]) => `${key}=${value}`).join("\n")}\n`,
    "utf8",
  );
}

function main(argv) {
  const profile = resolvePerformanceProfile([`--profile=${parseFlag(argv, "--profile") ?? "full"}`]);
  const output = path.resolve(root, parseFlag(argv, "--output") ?? ".performance-artifacts/fixture-identity.json");
  const identity = createPerformanceFixtureIdentity({ profile });
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, `${JSON.stringify(identity, null, 2)}\n`, "utf8");
  writeOutput({
    fixture_identity: identity.fixtureIdentity,
    fixture_cache_key: identity.fixtureCacheKey,
    fixture_format_version: identity.fixtureFormatVersion,
    fixture_schema_version: identity.schemaVersion,
  });
  console.log(`[perf-prepare] fixture-identity=${identity.fixtureIdentity}`);
}

if (import.meta.url === `file://${process.argv[1]?.replaceAll("\\", "/")}` || process.argv[1]?.endsWith("performanceFixtureIdentity.mjs")) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}
