import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import {
  getFixtureWorkingFilesForSuites,
  getPrecompileTargetsForSuites,
  PERFORMANCE_SUITE_NAMES,
  resolvePerformanceSuite,
} from "./performanceManifest.mjs";
import { resolvePerformanceProfile } from "./performanceProfile.mjs";
import {
  createFixtureManifest,
  manifestTargetPath,
  sha256File,
  validateBinaryManifest,
  validateFixtureManifest,
  writeJson,
} from "./performanceArtifactManifest.mjs";
import {
  createPerformanceFixtureIdentity,
  PERFORMANCE_FIXTURE_FORMAT_VERSION,
  PERFORMANCE_FIXTURE_SCHEMA_VERSION,
} from "./performanceFixtureIdentity.mjs";
import { createPerformanceBuildIdentity } from "./performanceBuildIdentity.mjs";
import { runPreparedTestBinary } from "./runPreparedPerformanceBinary.mjs";

const root = process.cwd();

function parseFlag(argv, name) {
  const index = argv.findIndex((argument) => argument === name || argument.startsWith(`${name}=`));
  if (index < 0) return undefined;
  const argument = argv[index];
  if (argument.startsWith(`${name}=`)) return argument.slice(name.length + 1);
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value.`);
  return value;
}

function resolveSuites(argv) {
  if (argv.includes("--all")) return [...PERFORMANCE_SUITE_NAMES];
  const value = parseFlag(argv, "--suites")
    ?? process.env.PERF_SUITES
    ?? PERFORMANCE_SUITE_NAMES.filter((suite) => {
      const key = suite.replaceAll("-", "_").replaceAll("/", "_");
      return process.env[`PERF_${key.toUpperCase()}`] === "true";
    }).join(",");
  if (!value) throw new Error("Specify --suites=<comma-separated names> or --all.");
  const suites = [...new Set(value.split(",").map((item) => item.trim()).filter(Boolean))];
  if (suites.length === 0) throw new Error("At least one performance suite is required.");
  for (const suite of suites) resolvePerformanceSuite([`--suite=${suite}`]);
  return suites;
}

function resolveProfile(argv) {
  const value = parseFlag(argv, "--profile");
  return resolvePerformanceProfile(value ? [`--profile=${value}`] : []);
}

function resolvePath(argv, name, fallback) {
  return path.resolve(root, parseFlag(argv, name) ?? fallback);
}

function currentCommit() {
  return process.env.PERF_CHECKOUT_SHA
    || process.env.GITHUB_SHA
    || execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
}

function writeOutput(values) {
  if (!process.env.GITHUB_OUTPUT) return;
  fs.appendFileSync(
    process.env.GITHUB_OUTPUT,
    `${Object.entries(values).map(([key, value]) => `${key}=${value}`).join("\n")}\n`,
    "utf8",
  );
}

function createManifest({ profile, suites, identity, requiredFiles, cacheRoot }) {
  const files = {};
  for (const relativePath of requiredFiles) {
    const filePath = path.join(cacheRoot, relativePath);
    if (!fs.existsSync(filePath)) throw new Error(`Prepared fixture is missing: ${filePath}`);
    files[relativePath] = { size: fs.statSync(filePath).size, sha256: sha256File(filePath) };
  }
  return createFixtureManifest({
    commit: currentCommit(),
    generatedFromCommit: currentCommit(),
    profile,
    suites,
    schemaVersion: PERFORMANCE_FIXTURE_SCHEMA_VERSION,
    fixtureFormatVersion: PERFORMANCE_FIXTURE_FORMAT_VERSION,
    fixtureIdentity: identity.fixtureIdentity,
    fixtureType: "file-library-sqlite-working-copies",
    rowCounts: identity.rowCounts,
    cacheScope: "fixture-cache",
    files,
  });
}

function main(argv) {
  const profile = resolveProfile(argv);
  const suites = resolveSuites(argv);
  const cacheBaseRoot = resolvePath(
    argv,
    "--cache-root",
    process.env.ZC_PERF_FIXTURE_CACHE_ROOT ?? ".tmp-performance-fixtures/cache",
  );
  const binariesRoot = resolvePath(argv, "--prepared-binaries", ".performance-artifacts/binaries");
  const requiredFiles = getFixtureWorkingFilesForSuites(suites, profile);
  if (requiredFiles.length === 0) {
    console.log(`[perf-prepare] no reusable fixtures required for suites=${suites.join(",")} profile=${profile}`);
    writeOutput({ fixture_cache_hit: "true", fixture_format_version: PERFORMANCE_FIXTURE_FORMAT_VERSION });
    return;
  }

  const identity = createPerformanceFixtureIdentity({ profile });
  const requestedIdentity = parseFlag(argv, "--fixture-identity");
  if (requestedIdentity && requestedIdentity !== identity.fixtureIdentity) {
    throw new Error(`Fixture identity differs from the workflow key: expected ${requestedIdentity}, got ${identity.fixtureIdentity}`);
  }
  const cacheRoot = path.join(cacheBaseRoot, identity.fixtureIdentity);
  fs.mkdirSync(cacheRoot, { recursive: true });
  const manifestPath = path.join(cacheRoot, "manifest.json");
  let cacheHit = false;
  if (fs.existsSync(manifestPath)) {
    try {
      validateFixtureManifest(cacheRoot, {
        expectedProfile: profile,
        expectedFixtureIdentity: identity.fixtureIdentity,
        expectedFixtureType: "file-library-sqlite-working-copies",
        expectedSchemaVersion: PERFORMANCE_FIXTURE_SCHEMA_VERSION,
        expectedFixtureFormatVersion: PERFORMANCE_FIXTURE_FORMAT_VERSION,
        expectedRowCounts: identity.rowCounts,
        expectedCacheScope: "fixture-cache",
        requiredFiles,
      });
      cacheHit = true;
    } catch (error) {
      console.log(`[perf-prepare] fixture-cache=invalid reason=${error instanceof Error ? error.message : String(error)}`);
      fs.rmSync(cacheRoot, { recursive: true, force: true });
      fs.mkdirSync(cacheRoot, { recursive: true });
    }
  }
  if (!cacheHit) {
    const builderManifestRoot = path.join(binariesRoot, "_prepare");
    const binaryIdentity = createPerformanceBuildIdentity({
      profile,
      suiteNames: suites,
      targetKeys: getPrecompileTargetsForSuites(suites).map((target) => target.targetKey),
    });
    const binaryManifest = validateBinaryManifest(builderManifestRoot, {
      expectedCommit: currentCommit(),
      expectedProfile: profile,
      expectedBuildIdentity: binaryIdentity.buildIdentity,
      expectedCargoLockSha256: binaryIdentity.cargoLockSha256,
      requiredTargets: ["fixtureBuilder"],
    });
    const builder = manifestTargetPath(builderManifestRoot, binaryManifest, "fixtureBuilder");
    const started = Date.now();
    runPreparedTestBinary({
      executable: builder,
      testName: "build_requested_performance_fixtures",
      cwd: root,
      env: {
        ...process.env,
        ZC_PERF_FIXTURE_ROOT: cacheRoot,
        ZC_PERF_FIXTURE_WORKING_ROOT: cacheRoot,
        ZC_PERF_FIXTURE_REQUIRED: "0",
        ZC_PERF_FIXTURE_BUILD: "1",
        ZC_PERFORMANCE_PROFILE: profile,
      },
      timeoutMs: 45 * 60 * 1000,
    });
    console.log(`[perf-prepare] phase=fixture-generation cache=miss ms=${Date.now() - started}`);
    const manifest = createManifest({ profile, suites, identity, requiredFiles, cacheRoot });
    writeJson(manifestPath, manifest);
    validateFixtureManifest(cacheRoot, {
      expectedProfile: profile,
      expectedFixtureIdentity: identity.fixtureIdentity,
      expectedFixtureType: "file-library-sqlite-working-copies",
      expectedSchemaVersion: PERFORMANCE_FIXTURE_SCHEMA_VERSION,
      expectedFixtureFormatVersion: PERFORMANCE_FIXTURE_FORMAT_VERSION,
      expectedRowCounts: identity.rowCounts,
      expectedCacheScope: "fixture-cache",
      requiredFiles,
    });
  }

  console.log(`[perf-prepare] fixture-cache=${cacheHit ? "hit" : "miss"}`);
  writeOutput({
    fixture_cache_hit: cacheHit ? "true" : "false",
    fixture_cache_key: [
      "fylura-perf-fixture",
      identity.runner.os,
      identity.runner.arch,
      profile,
      identity.fixtureIdentity,
    ].join("-"),
    fixture_identity: identity.fixtureIdentity,
    fixture_format_version: PERFORMANCE_FIXTURE_FORMAT_VERSION,
    fixture_files: requiredFiles.length,
  });
  console.log(`[perf-prepare] phase=fixture-validation files=${requiredFiles.length} ms=0`);
}

try {
  main(process.argv.slice(2));
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
