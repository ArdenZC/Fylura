import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import {
  getFixtureWorkingFiles,
  getPerformanceBenchmarks,
  getPrecompileTargetsForSuites,
  getRequiredBinaryKeys,
  PERFORMANCE_BUILD_FEATURES,
  resolvePerformanceSuite,
} from "./performanceManifest.mjs";
import { resolvePerformanceProfile } from "./performanceProfile.mjs";
import {
  createPerformanceFixtureIdentity,
  PERFORMANCE_FIXTURE_SCHEMA_VERSION,
} from "./performanceFixtureIdentity.mjs";
import { createPerformanceBuildIdentity } from "./performanceBuildIdentity.mjs";
import {
  manifestTargetPath,
  validateBinaryManifest,
  validateFixtureManifest,
} from "./performanceArtifactManifest.mjs";
import { runPreparedTestBinary } from "./runPreparedPerformanceBinary.mjs";

const root = process.cwd();

function parseValue(argv, name) {
  const index = argv.findIndex((argument) => argument === name || argument.startsWith(`${name}=`));
  if (index < 0) return undefined;
  const argument = argv[index];
  if (argument.startsWith(`${name}=`)) return argument.slice(name.length + 1);
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value.`);
  return value;
}

function parseArguments(argv) {
  const allowed = [
    "--suite",
    "--profile",
    "--benchmark-id",
    "--prepared-binaries",
    "--fixture-root",
    "--build-identity",
    "--fixture-identity",
    "--prepare-missing-fixtures",
  ];
  for (const argument of argv) {
    if (argument === "--prepare-missing-fixtures") continue;
    if (!allowed.some((name) => argument === name || argument.startsWith(`${name}=`))) {
      throw new Error(`Unknown performance suite argument: ${argument}`);
    }
  }
  const suiteValue = parseValue(argv, "--suite");
  const profileValue = parseValue(argv, "--profile");
  return {
    suite: resolvePerformanceSuite(suiteValue ? [`--suite=${suiteValue}`] : []),
    profile: resolvePerformanceProfile(profileValue ? [`--profile=${profileValue}`] : []),
    benchmarkId: parseValue(argv, "--benchmark-id"),
    preparedBinaries: parseValue(argv, "--prepared-binaries"),
    fixtureRoot: parseValue(argv, "--fixture-root"),
    buildIdentity: parseValue(argv, "--build-identity") ?? process.env.PERF_BINARY_BUILD_IDENTITY,
    fixtureIdentity: parseValue(argv, "--fixture-identity") ?? process.env.PERF_FIXTURE_IDENTITY,
    prepareMissing: argv.includes("--prepare-missing-fixtures"),
  };
}

function selectBenchmarks(suite, profile, benchmarkId) {
  const benchmarks = getPerformanceBenchmarks(suite, profile);
  if (benchmarkId === undefined) return benchmarks;
  const selected = benchmarks.filter((benchmark) => benchmark.id === benchmarkId);
  if (selected.length !== 1) {
    throw new Error(`Unknown or ambiguous performance benchmark for ${suite} (${profile}): ${benchmarkId}`);
  }
  return selected;
}

function currentCommit() {
  return process.env.PERF_CHECKOUT_SHA
    || process.env.GITHUB_SHA
    || execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
}

function cargoLockSha256() {
  return crypto.createHash("sha256")
    .update(fs.readFileSync(path.join(root, "src-tauri", "Cargo.lock")))
    .digest("hex");
}

function runPreparation(suite, profile) {
  const artifactRoot = path.join(root, ".performance-artifacts");
  const fixtureBaseRoot = path.join(root, ".tmp-performance-fixtures", "cache");
  const fixtureIdentity = createPerformanceFixtureIdentity({ profile }).fixtureIdentity;
  const commands = [
    ["preparePerformanceBinaries.mjs", [
      `--suites=${suite}`,
      `--profile=${profile}`,
      `--cache-root=${path.join(root, ".performance-cache", "binaries")}`,
      `--output=${path.join(artifactRoot, "binaries")}`,
    ]],
    ["preparePerformanceFixtures.mjs", [
      `--suites=${suite}`,
      `--profile=${profile}`,
      `--prepared-binaries=${path.join(artifactRoot, "binaries")}`,
      `--cache-root=${fixtureBaseRoot}`,
    ]],
  ];
  for (const [script, args] of commands) {
    const result = spawnSync(process.execPath, [path.join(root, "scripts", script), ...args], {
      cwd: root,
      env: process.env,
      stdio: "inherit",
      windowsHide: true,
    });
    if (result.error) throw new Error(`${script} failed to start: ${result.error.message}`);
    if (result.status !== 0) throw new Error(`${script} failed with exit code ${result.status}.`);
  }
  return {
    preparedBinaries: path.join(artifactRoot, "binaries", suite),
    fixtureRoot: path.join(fixtureBaseRoot, fixtureIdentity),
  };
}

function resolveFixtureRoot(baseRoot, fixtureIdentity) {
  const identityRoot = path.join(baseRoot, fixtureIdentity);
  return fs.existsSync(path.join(identityRoot, "manifest.json")) ? identityRoot : baseRoot;
}

function appendSummary(suite, profile, elapsedMs) {
  if (!process.env.GITHUB_STEP_SUMMARY) return;
  fs.appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `- ${suite} (${profile}) wall-clock: ${(elapsedMs / 1000).toFixed(3)}s\n`,
    "utf8",
  );
}

function main(argv) {
  const selection = parseArguments(argv);
  const { suite, profile } = selection;
  const benchmarks = selectBenchmarks(suite, profile, selection.benchmarkId);
  let preparedBinaries = selection.preparedBinaries
    ? path.resolve(root, selection.preparedBinaries)
    : undefined;
  let fixtureRoot = selection.fixtureRoot
    ? path.resolve(root, selection.fixtureRoot)
    : undefined;
  if (!preparedBinaries) {
    const isCi = process.env.CI === "true" || process.env.GITHUB_ACTIONS === "true";
    if (isCi && !selection.prepareMissing) {
      throw new Error("CI performance shards require --prepared-binaries; refusing to build in the consumer shard.");
    }
    if (!selection.prepareMissing) {
      throw new Error("Prepared binaries are required. Use --prepare-missing-fixtures for explicit local preparation.");
    }
    const prepared = runPreparation(suite, profile);
    preparedBinaries = prepared.preparedBinaries;
    fixtureRoot = prepared.fixtureRoot;
  }

  const expectedBuildIdentity = selection.buildIdentity
    ?? createPerformanceBuildIdentity({
      profile,
      suiteNames: [suite],
      features: PERFORMANCE_BUILD_FEATURES,
      targetKeys: getPrecompileTargetsForSuites([suite]).map((target) => target.targetKey),
    }).buildIdentity;
  const binaryManifest = validateBinaryManifest(preparedBinaries, {
    expectedCommit: currentCommit(),
    expectedProfile: profile,
    expectedBuildIdentity,
    expectedCargoLockSha256: cargoLockSha256(),
    expectedSuites: [suite],
    requiredTargets: getRequiredBinaryKeys(suite),
  });
  const requiredFixtures = getFixtureWorkingFiles(suite, profile);
  if (requiredFixtures.length > 0) {
    if (!fixtureRoot) throw new Error(`Suite ${suite} requires --fixture-root.`);
    const fixtureIdentity = selection.fixtureIdentity ?? createPerformanceFixtureIdentity({ profile }).fixtureIdentity;
    fixtureRoot = resolveFixtureRoot(fixtureRoot, fixtureIdentity);
    validateFixtureManifest(fixtureRoot, {
      expectedProfile: profile,
      expectedFixtureIdentity: fixtureIdentity,
      expectedFixtureType: "file-library-sqlite-working-copies",
      expectedSchemaVersion: PERFORMANCE_FIXTURE_SCHEMA_VERSION,
      expectedFixtureFormatVersion: 1,
      expectedCacheScope: "fixture-cache",
      requiredFiles: requiredFixtures,
    });
  }

  const benchmarkEnv = {
    ...process.env,
    ZC_PERFORMANCE_PROFILE: profile,
    ZC_PERF_SUITE: suite,
    ZC_FTS_FULL_PROFILE: String(profile === "full"),
    ZC_PERF_FIXTURE_REQUIRED: requiredFixtures.length > 0 ? "1" : "0",
    ...(fixtureRoot ? {
      ZC_PERF_FIXTURE_ROOT: fixtureRoot,
      ZC_PERF_PREPARED_WORKING_COPY: "1",
    } : {}),
    ...(suite === "workspace-foundation" || suite === "preview-platform" ? {
      ZC_PERF_WORKSPACE_FIXTURE_ROOT: selection.fixtureRoot
        ? fixtureRoot
        : path.resolve(root, ".tmp-performance-fixtures", suite),
    } : {}),
  };
  const started = Date.now();
  for (const benchmark of benchmarks) {
    const benchmarkStarted = Date.now();
    console.log(`Running ${benchmark.label} from prepared binary...`);
    runPreparedTestBinary({
      executable: manifestTargetPath(preparedBinaries, binaryManifest, benchmark.targetKey),
      testName: benchmark.testName,
      ignored: benchmark.ignored,
      testThreads: benchmark.testThreads,
      cwd: root,
      env: { ...benchmarkEnv, ...benchmark.env },
      timeoutMs: 45 * 60 * 1000,
    });
    console.log(
      `[perf-phase] suite=${suite} phase=benchmark id=${benchmark.id} ms=${Date.now() - benchmarkStarted}`,
    );
  }
  const elapsedMs = Date.now() - started;
  console.log(`${suite} ${profile} performance suite passed in ${(elapsedMs / 1000).toFixed(3)}s.`);
  appendSummary(suite, profile, elapsedMs);
}

try {
  main(process.argv.slice(2));
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
