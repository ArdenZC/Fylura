import { describe, expect, it } from "vitest";
import { buildPreflightPlan, parseOptions } from "../scripts/devPreflight.mjs";
import { formatFailureDigest } from "../scripts/ciFailureDigest.mjs";
import { parseCacheLog, summarizeCiRuns, formatCiMetrics } from "../scripts/ciRunMetrics.mjs";

describe("opt-in developer preflight", () => {
  it("selects documentation checks without Rust or frontend work", () => {
    const plan = buildPreflightPlan(["docs/project/ROADMAP.md"]);
    expect(plan.commands.map((x) => x.label)).toEqual(["Documentation contract"]);
    expect(plan.scope.docs_only).toBe(true);
  });
  it("runs frontend checks for a UI change", () => {
    const plan = buildPreflightPlan(["src/views/fileLibrary/index.tsx"]);
    expect(plan.commands.map((x) => x.label)).toEqual(["TypeScript", "Frontend tests", "Frontend build"]);
    expect(plan.scope.rust_changed).toBe(false);
  });
  it("includes Clippy and only a requested focused Rust test", () => {
    const plan = buildPreflightPlan(["src-tauri/src/global_index/repository.rs"], { rustTest: "regression" });
    expect(plan.commands.some((x) => x.label === "Rust Clippy")).toBe(true);
    expect(plan.commands.find((x) => x.label === "Focused Rust regression")?.args.at(-1)).toBe("regression");
    expect(plan.commands.some((x) => x.label === "Rust regression suite")).toBe(false);
    expect(plan.warnings.some((x) => x.includes("High-risk path"))).toBe(true);
  });
  it("uses the full Rust suite only on explicit request", () => {
    const plan = buildPreflightPlan(["src-tauri/src/db/schema.rs"], { full: true });
    expect(plan.commands.some((x) => x.label === "Rust regression suite")).toBe(true);
    expect(plan.warnings.some((x) => x.includes("runtime tests not run"))).toBe(false);
  });
  it("checks native workspace and rejects conflicting options", () => {
    expect(buildPreflightPlan(["src-tauri/native/preview-representation/src/lib.rs"]).commands.some((x) => x.label === "Native Rust tests")).toBe(true);
    expect(() => parseOptions(["--full", "--rust-test=a"])).toThrow();
    expect(() => parseOptions(["--invalid"])).toThrow();
  });
  it("handles an empty diff without executing unrelated checks", () => {
    expect(buildPreflightPlan([]).commands).toEqual([]);
  });
});
describe("GitHub CI failure digest", () => {
  it("shows original failed steps before downstream Quality aggregates", () => {
    const result = formatFailureDigest({
      workflowName: "CI", conclusion: "failure", headSha: "abc",
      jobs: [
        { name: "Quality (windows-latest)", conclusion: "failure", steps: [{ name: "Verify validation plan", conclusion: "failure" }] },
        { name: "Rust quality (windows-latest)", conclusion: "failure", steps: [{ name: "Rust clippy", conclusion: "failure" }] },
        { name: "Frontend", conclusion: "skipped" },
      ],
    });
    expect(result.indexOf("Rust quality")).toBeLessThan(result.indexOf("Quality (windows-latest)"));
    expect(result).toContain("step: Rust clippy");
    expect(result).toContain("possible downstream aggregate");
    expect(result).not.toContain("- Frontend: skipped");
  });
  it("does not claim in-progress or skipped work passed", () => {
    const result = formatFailureDigest({ status: "in_progress", jobs: [{ name: "A", conclusion: "skipped" }] });
    expect(result).toContain("No failed jobs reported");
    expect(result).toContain("Do not interpret skipped checks as PASS");
  });
});


describe("opt-in CI measurement (no new required gate)", () => {
  it("keeps failures and skipped outcomes separate from successful runs", () => {
    const summary = summarizeCiRuns([
      { databaseId: 1, conclusion: "success", jobs: [
        { name: "Frontend", conclusion: "success", startedAt: "2026-10-29T01:00:00Z", completedAt: "2026-10-29T01:04:00Z" },
        { name: "Native", conclusion: "skipped" }
      ] },
      { databaseId: 2, conclusion: "failure", jobs: [
        { name: "Rust quality", conclusion: "failure", startedAt: "2026-10-29T02:00:00Z", completedAt: "2026-10-29T02:15:00Z" }
      ] },
      { databaseId: 3, status: "in_progress", jobs: [] }
    ]);
    expect(summary.outcomes).toEqual({ success: 1, failure: 1, cancelled: 0, other: 1 });
    expect(summary.failures).toEqual([["Rust quality", 1]]);
    expect(summary.slowJobs.map((x) => x.name)).toEqual(["Rust quality", "Frontend"]);
    expect(formatCiMetrics(summary)).toContain("Cache state (explicit Performance / Prepare log evidence only)");
    expect(formatCiMetrics(summary)).toContain("unknown; supply --cache-log");
    expect(formatCiMetrics(summary)).toContain("skipped jobs are never PASS");
  });

  it("only counts explicit perf-prepare cache hit/miss logs, never guesses from job success", () => {
    const cache = parseCacheLog(
      "[perf-prepare] rust-cache=miss\n[perf-prepare] prepared-binary-cache=hit\n" +
      "[perf-prepare] fixture-cache=hit\nWorkflow job SUCCESS"
    );
    expect(cache).toEqual({
      "rust-cache": "miss",
      "prepared-binary-cache": "hit",
      "fixture-cache": "hit"
    });
    expect(formatCiMetrics(summarizeCiRuns([]), cache)).toContain("- rust-cache: miss");
    expect(parseCacheLog("Cache exists? not proven.")).toEqual({});
  });
});
