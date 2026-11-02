#!/usr/bin/env node
// Opt-in evidence collector for Workspace Foundation resource calibration.
// Not a CI gate and NEVER a replacement for the existing native HARD PASS.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { buildPreparedTestArgs } from "./runPreparedPerformanceBinary.mjs";

export const RESOURCE_TEST_NAME =
  "file_workspace::integration::performance::steady_state::resource_and_registry_steady_state_after_browse_preview_switches";
const PREFIX = "[zc-perf] ";
const REQUIRED = ["resource_observations", "resource_epoch_trend", "resource_registry_steady_state"];

export function extractResourceEvidence(output) {
  const records = new Map();
  for (const line of String(output).split(/\r?\n/)) {
    const offset = line.indexOf(PREFIX);
    if (offset < 0) continue;
    let record;
    try {
      record = JSON.parse(line.slice(offset + PREFIX.length).trim());
    } catch {
      throw new Error("Malformed [zc-perf] JSON cannot be used as calibration evidence");
    }
    if (record.suite !== "workspace-foundation" || !REQUIRED.includes(record.scenario)) continue;
    if (records.has(record.scenario)) {
      throw new Error("Duplicate native resource scenario: " + record.scenario);
    }
    records.set(record.scenario, record);
  }
  for (const name of REQUIRED) {
    if (!records.has(name)) throw new Error("Missing native resource scenario: " + name);
  }
  const observed = records.get("resource_observations");
  const trend = records.get("resource_epoch_trend");
  const owners = records.get("resource_registry_steady_state");
  if (observed.schema !== 1 || trend.schema !== 1 || owners.schema !== 1) {
    throw new Error("Unexpected resource metric schema");
  }
  if (observed.platform !== trend.platform || observed.platform !== owners.platform) {
    throw new Error("Resource metric platforms do not agree");
  }
  const n = observed.epoch_count;
  if (!Number.isInteger(n) || n !== 5 || trend.epoch_count !== n || owners.epoch_count !== n) {
    throw new Error("Unexpected epoch count: expected five measured epochs");
  }
  for (const key of ["settled_rss_samples", "settled_private_committed_samples", "settled_handle_samples", "settled_fd_samples"]) {
    const series = observed[key];
    if (!Array.isArray(series) || series.length !== n ||
        !series.every((v) => v === null || (Number.isSafeInteger(v) && v >= 0))) {
      throw new Error("Invalid measured series: " + key);
    }
  }
  if (![true, false].includes(trend.hard_resource_signal_available) ||
      ![true, false].includes(trend.hard_resource_growth) ||
      !["HARD PASS", "BLOCKED"].includes(trend.classification)) {
    throw new Error("Invalid native resource hard-gate evidence");
  }
  const net = (series) => {
    if (series[0] === null || series.at(-1) === null) return null;
    return series.at(-1) - series[0];
  };
  return {
    platform: observed.platform,
    epochs: n,
    resourceTrendClassification: trend.classification,
    registryClassification: owners.classification,
    hardSignalAvailable: trend.hard_resource_signal_available,
    hardResourceGrowth: trend.hard_resource_growth,
    rssBytes: observed.settled_rss_samples,
    privateCommittedBytes: observed.settled_private_committed_samples,
    handles: observed.settled_handle_samples,
    fileDescriptors: observed.settled_fd_samples,
    rssNetBytes: net(observed.settled_rss_samples),
    privateCommittedNetBytes: net(observed.settled_private_committed_samples),
    handleNet: net(observed.settled_handle_samples),
    fdNet: net(observed.settled_fd_samples),
    afterDisposeRegistryZero: owners.internal_registry_zero === true,
    previewCycles: owners.preview_cycles,
    thumbnailCycles: owners.thumbnail_cycles,
    targetSwitchCycles: owners.target_switch_cycles,
  };
}

export function summarizeCalibration(runs) {
  const complete = runs.filter((run) => run.evidence !== null);
  const blocked = complete.filter((run) => run.evidence.resourceTrendClassification === "BLOCKED");
  const hardPass = complete.filter((run) => run.evidence.resourceTrendClassification === "HARD PASS");
  const failures = runs.filter((run) => run.exitCode !== 0 || run.signal || run.error);
  const platforms = [...new Set(complete.map((run) => run.evidence.platform))];
  return {
    requestedRuns: runs.length,
    validEvidenceRuns: complete.length,
    nativeHardPassRuns: hardPass.length,
    nativeBlockedRuns: blocked.length,
    processFailureRuns: failures.length,
    platforms,
    fullyCollected: complete.length === runs.length && platforms.length === 1,
    allNativeHardPass: hardPass.length === runs.length && failures.length === 0,
    // Not a decision about allowed memory slope, leak, CI bypass or release.
    interpretation: "CALIBRATION_ONLY_NOT_PRODUCT_QUALIFICATION",
  };
}

function readArgs(argv) {
  const options = {};
  for (const arg of argv) {
    if (arg === "--help") return { help: true };
    const match = /^--(binary|runs|output|source-sha)=(.+)$/.exec(arg);
    if (!match || options[match[1]] !== undefined) throw new Error("Unknown or duplicate option: " + arg);
    options[match[1]] = match[2];
  }
  if (!options.binary) throw new Error("--binary=PATH is required (already-prepared native test executable)");
  const runs = options.runs === undefined ? 3 : Number(options.runs);
  if (!Number.isInteger(runs) || runs < 3 || runs > 8) throw new Error("--runs must be an integer from 3 to 8");
  return {
    binary: path.resolve(options.binary),
    runs,
    output: path.resolve(options.output ?? ".tmp-resource-calibration/workspace-resource.json"),
    sourceSha: options["source-sha"] ?? null,
  };
}

async function sha256OfFile(file) {
  const hash = crypto.createHash("sha256");
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

export async function collectResourceCalibration({
  binary, runs = 3, output, sourceSha = null, spawnImpl = spawnSync,
}) {
  if (!Number.isInteger(runs) || runs < 3 || runs > 8) throw new Error("3–8 independent runs required");
  const executable = path.resolve(binary);
  const stat = fs.statSync(executable);
  if (!stat.isFile()) throw new Error("Native binary must be an existing regular file");
  const hash = await sha256OfFile(executable);
  const list = spawnImpl(executable, [RESOURCE_TEST_NAME, "--list", "--exact", "--ignored"], {
    encoding: "utf8", timeout: 30000, maxBuffer: 32 * 1024 * 1024, windowsHide: true,
  });
  const names = String(list.stdout ?? "").split(/\r?\n/)
    .map((s) => s.trim()).filter((s) => s.endsWith(": test"))
    .map((s) => s.slice(0, -": test".length));
  if (list.error || list.signal || list.status !== 0 || names.length !== 1 || names[0] !== RESOURCE_TEST_NAME) {
    throw new Error("Prepared binary does not list exactly the expected ignored resource test");
  }
  const results = [];
  for (let i = 0; i < runs; i++) {
    const start = Date.now();
    const child = spawnImpl(executable, buildPreparedTestArgs(RESOURCE_TEST_NAME, {
      ignored: true, testThreads: 1,
    }), {
      encoding: "utf8", timeout: 10 * 60 * 1000, maxBuffer: 32 * 1024 * 1024,
      windowsHide: true, env: { ...process.env, RUST_TEST_THREADS: "1" },
    });
    const outputText = String(child.stdout ?? "") + "\n" + String(child.stderr ?? "");
    let evidence = null;
    let error = child.error?.message ?? null;
    try {
      evidence = extractResourceEvidence(outputText);
    } catch (e) {
      error = error ? error + "; " + e.message : e.message;
    }
    results.push({
      run: i + 1,
      elapsedMs: Date.now() - start,
      exitCode: child.status,
      signal: child.signal ?? null,
      error,
      evidence,
      // Preserve bounded diagnostics, not complete runner logs or user data.
      stderrTail: String(child.stderr ?? "").slice(-1200),
    });
    console.log("[resource-calibration] run " + (i + 1) + "/" + runs +
      " exit=" + child.status + " evidence=" + (evidence?.resourceTrendClassification ?? "MISSING"));
  }
  const result = {
    schema: 1,
    purpose: "B_PLAN_NATIVE_RESOURCE_BASELINE_ONLY",
    generatedAt: new Date().toISOString(),
    binary: { sha256: hash, bytes: stat.size, sourceSha },
    host: { platform: process.platform, arch: process.arch, osRelease: os.release() },
    testName: RESOURCE_TEST_NAME,
    runs: results,
    summary: summarizeCalibration(results),
    warning: "Native BLOCKED and process failures remain failures. These observations cannot change the five-epoch CI hard gate, certify a release, or prove absence of a leak.",
  };
  if (output) {
    fs.mkdirSync(path.dirname(output), { recursive: true });
    fs.writeFileSync(output, JSON.stringify(result, null, 2) + "\n");
  }
  return result;
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === new URL(import.meta.url).pathname;
// On Windows URL pathname includes a leading slash and URL encoding; normalize through fileURLToPath.
if (process.argv[1] && (isMain || path.resolve(process.argv[1]) === (await import("node:url")).fileURLToPath(import.meta.url))) {
  try {
    const options = readArgs(process.argv.slice(2));
    if (options.help) {
      console.log("node scripts/calibrateWorkspaceResourceTrend.mjs --binary=PATH [--runs=3] [--output=PATH] [--source-sha=COMMIT]");
    } else {
      const report = await collectResourceCalibration(options);
      console.log("[resource-calibration] " + JSON.stringify(report.summary));
      if (!report.summary.allNativeHardPass) process.exitCode = 2;
    }
  } catch (error) {
    console.error("[resource-calibration] " + error.message);
    process.exitCode = 1;
  }
}
