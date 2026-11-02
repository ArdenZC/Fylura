import { afterEach, describe, expect, it } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  RESOURCE_TEST_NAME,
  collectResourceCalibration,
  extractResourceEvidence,
  summarizeCalibration,
} from "../scripts/calibrateWorkspaceResourceTrend.mjs";

const tempDirs: string[] = [];
afterEach(() => {
  for (const directory of tempDirs.splice(0)) fs.rmSync(directory, { recursive: true, force: true });
});

function metric(scenario: string, values: number[], classification = "HARD PASS") {
  const common = {
    schema: 1, suite: "workspace-foundation", scenario, platform: "macos",
    epoch_count: 5, classification,
  };
  if (scenario === "resource_observations") {
    return {
      ...common,
      settled_rss_samples: values,
      settled_private_committed_samples: [null, null, null, null, null],
      settled_handle_samples: [null, null, null, null, null],
      settled_fd_samples: [8, 8, 8, 8, 8],
    };
  }
  if (scenario === "resource_epoch_trend") {
    return { ...common, hard_resource_signal_available: true, hard_resource_growth: classification === "BLOCKED" };
  }
  return { ...common, internal_registry_zero: true, preview_cycles: 100, thumbnail_cycles: 100, target_switch_cycles: 100 };
}

function logFor(values: number[], classification = "HARD PASS") {
  return ["resource_observations", "resource_epoch_trend", "resource_registry_steady_state"]
    .map((scenario) => "2026-11-01T00:00:00Z [zc-perf] " +
      JSON.stringify(metric(scenario, values, scenario === "resource_epoch_trend" ? classification : "HARD PASS")))
    .join("\n");
}

describe("B-plan same-binary native resource calibration", () => {
  it("preserves the native blocked classification even for an 80 KiB RSS rise", () => {
    const evidence = extractResourceEvidence(logFor([
      38_027_264, 38_043_648, 38_060_032, 38_076_416, 38_109_184,
    ], "BLOCKED"));
    expect(evidence.rssNetBytes).toBe(81_920);
    expect(evidence.resourceTrendClassification).toBe("BLOCKED");
    expect(evidence.afterDisposeRegistryZero).toBe(true);
    expect(evidence.fdNet).toBe(0);
    expect(evidence.previewCycles).toBe(100);
  });

  it("rejects incomplete, duplicate or inconsistent evidence rather than inventing PASS", () => {
    const good = logFor([100, 100, 100, 100, 100]);
    expect(() => extractResourceEvidence(good.replace(/.*resource_registry_steady_state.*\n?/, "")))
      .toThrow("Missing native resource scenario");
    expect(() => extractResourceEvidence(good + "\n" + good)).toThrow("Duplicate native");
    expect(() => extractResourceEvidence(good.replace('"epoch_count":5', '"epoch_count":4')))
      .toThrow("Unexpected epoch count");
    expect(() => extractResourceEvidence("plain text")).toThrow("Missing native resource scenario");
  });

  it("runs three fresh processes using the same binary and records native failures", async () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "fylura-resource-calibration-"));
    tempDirs.push(directory);
    const binary = path.join(directory, "native-test-binary");
    fs.writeFileSync(binary, "synthetic executable identity for spawn mock");
    const output = path.join(directory, "report.json");
    let count = 0;
    const run = (executable: string, args: string[]) => {
      expect(executable).toBe(binary);
      if (args.includes("--list")) return { status: 0, stdout: RESOURCE_TEST_NAME + ": test\n" };
      count++;
      expect(args).toContain("--ignored");
      expect(args).toContain("--test-threads=1");
      const blocked = count === 2;
      return {
        status: blocked ? 101 : 0,
        stdout: logFor(blocked ? [10, 20, 30, 40, 50] : [10, 10, 10, 10, 10],
          blocked ? "BLOCKED" : "HARD PASS"),
        stderr: blocked ? "intentional native assertion failed" : "",
      };
    };
    const result = await collectResourceCalibration({
      binary, runs: 3, output, sourceSha: "abc123", spawnImpl: run as never,
    });
    expect(count).toBe(3);
    expect(result.binary.sourceSha).toBe("abc123");
    expect(result.binary.sha256).toMatch(/^[a-f0-9]{64}$/);
    expect(result.summary.nativeHardPassRuns).toBe(2);
    expect(result.summary.nativeBlockedRuns).toBe(1);
    expect(result.summary.processFailureRuns).toBe(1);
    expect(result.summary.allNativeHardPass).toBe(false);
    expect(result.summary.interpretation).toBe("CALIBRATION_ONLY_NOT_PRODUCT_QUALIFICATION");
    expect(fs.readFileSync(output, "utf8")).toContain("B_PLAN_NATIVE_RESOURCE_BASELINE_ONLY");
  });

  it("does not call partially captured or inconsistent runs qualified", () => {
    const summary = summarizeCalibration([
      { exitCode: 0, signal: null, error: null, evidence: { platform: "macos", resourceTrendClassification: "HARD PASS" } },
      { exitCode: 101, signal: null, error: "Missing native resource scenario", evidence: null },
      { exitCode: 0, signal: null, error: null, evidence: { platform: "macos", resourceTrendClassification: "HARD PASS" } },
    ]);
    expect(summary.fullyCollected).toBe(false);
    expect(summary.allNativeHardPass).toBe(false);
  });
});
