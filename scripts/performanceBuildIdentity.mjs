import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { PERFORMANCE_ARTIFACT_FORMAT_VERSION } from "./performanceArtifactManifest.mjs";
import {
  getPrecompileTargetsForSuites,
  PERFORMANCE_BUILD_FEATURES,
  PERFORMANCE_SUITE_NAMES,
  resolvePerformanceSuite,
} from "./performanceManifest.mjs";

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

function resolveProfile(argv) {
  const value = parseFlag(argv, "--profile") ?? "full";
  if (!matchesProfile(value)) throw new Error(`Unsupported performance profile: ${value}`);
  return value;
}

function matchesProfile(value) {
  return value === "full" || value === "extended";
}

function resolveFeatures(argv) {
  return parseFlag(argv, "--features") ?? process.env.PERF_FEATURES ?? PERFORMANCE_BUILD_FEATURES;
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

function rustVersion() {
  return execFileSync("rustc", ["-Vv"], { cwd: root, encoding: "utf8" }).trim();
}

function fileHash(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function filesUnder(directory) {
  if (!fs.existsSync(directory)) return [];
  const result = [];
  const visit = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const resolved = path.join(current, entry.name);
      if (entry.isDirectory()) visit(resolved);
      else result.push(resolved);
    }
  };
  visit(directory);
  return result;
}

function trackedBuildInputs() {
  const candidates = [
    path.join(root, "src-tauri", "Cargo.toml"),
    path.join(root, "src-tauri", "Cargo.lock"),
    path.join(root, "src-tauri", "build.rs"),
    path.join(root, ".cargo"),
    path.join(root, "src-tauri", ".cargo"),
    path.join(root, "rust-toolchain"),
    path.join(root, "rust-toolchain.toml"),
    path.join(root, "src-tauri", "tauri.conf.json"),
    path.join(root, "scripts", "performanceManifest.mjs"),
    path.join(root, "scripts", "windowsReleaseFeatures.mjs"),
    path.join(root, "scripts", "buildWindowsPackage.mjs"),
    path.join(root, "src-tauri", "src"),
    path.join(root, "src-tauri", "tests"),
  ];
  const files = [];
  for (const candidate of candidates) {
    if (!fs.existsSync(candidate)) continue;
    if (fs.statSync(candidate).isDirectory()) files.push(...filesUnder(candidate));
    else files.push(candidate);
  }
  return [...new Set(files)]
    .sort((left, right) => left.localeCompare(right))
    .map((filePath) => ({
      path: path.relative(root, filePath).replaceAll("\\", "/"),
      sha256: fileHash(filePath),
    }));
}

export function createPerformanceBuildIdentity({
  profile,
  features = PERFORMANCE_BUILD_FEATURES,
  targetKeys = [],
  suiteNames = [],
  rust = rustVersion(),
  runnerOs = process.env.RUNNER_OS ?? process.platform,
  runnerArch = process.env.RUNNER_ARCH ?? process.arch,
} = {}) {
  if (!matchesProfile(profile)) throw new Error(`Unsupported performance profile: ${profile}`);
  const inputs = trackedBuildInputs();
  const cargoLockSha256 = inputs.find((input) => input.path === "src-tauri/Cargo.lock")?.sha256;
  if (!cargoLockSha256) throw new Error("src-tauri/Cargo.lock is required for performance build identity.");
  const normalizedTargetKeys = [...new Set(targetKeys)].sort((left, right) => left.localeCompare(right));
  const normalizedSuiteNames = [...new Set(suiteNames)].sort((left, right) => left.localeCompare(right));
  const payload = {
    identityVersion: 2,
    runner: { os: runnerOs, arch: runnerArch },
    rustVersion: rust,
    cargoLockSha256,
    profile,
    features,
    suiteNames: normalizedSuiteNames,
    targetKeys: normalizedTargetKeys,
    artifactFormatVersion: PERFORMANCE_ARTIFACT_FORMAT_VERSION,
    inputs,
  };
  const buildIdentity = crypto
    .createHash("sha256")
    .update(JSON.stringify(payload))
    .digest("hex");
  return { buildIdentity, cargoLockSha256, rustVersion: rust, runner: payload.runner, payload };
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
  const profile = resolveProfile(argv);
  const features = resolveFeatures(argv);
  const suites = resolveSuites(argv);
  const targetKeys = getPrecompileTargetsForSuites(suites).map((target) => target.targetKey);
  const output = path.resolve(root, parseFlag(argv, "--output") ?? ".performance-artifacts/binary-build-identity.json");
  const identity = createPerformanceBuildIdentity({ profile, features, suiteNames: suites, targetKeys });
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, `${JSON.stringify(identity, null, 2)}\n`, "utf8");
  writeOutput({
    build_identity: identity.buildIdentity,
    cargo_lock_sha256: identity.cargoLockSha256,
    runner_os: identity.runner.os,
    runner_arch: identity.runner.arch,
  });
  console.log(`[perf-prepare] binary-build-identity=${identity.buildIdentity}`);
}

if (import.meta.url === `file://${process.argv[1]?.replaceAll("\\", "/")}` || process.argv[1]?.endsWith("performanceBuildIdentity.mjs")) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}
