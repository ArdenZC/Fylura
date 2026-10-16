import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { classifyCiScope } from "../scripts/classifyCiChanges.mjs";

function readWorkflow(relativePath: string) {
  return readFileSync(relativePath, "utf8").replace(/\r\n?/gu, "\n");
}

function route(changedPaths: string[], options: Parameters<typeof classifyCiScope>[0] = {}) {
  return classifyCiScope({ event: "pull_request", changedPaths, ...options });
}

function performanceFlags(scope: ReturnType<typeof classifyCiScope>) {
  return [
    scope.perf_search,
    scope.perf_scan_schema,
    scope.perf_library_content,
    scope.perf_intelligence,
    scope.perf_workspace_foundation,
    scope.perf_preview_platform,
  ];
}

describe("CI change routing", () => {
  it("routes documentation-only changes to the documentation gate", () => {
    const scope = route(["docs/ci.md"]);
    expect(scope.docs_only).toBe(true);
    expect(scope.frontend_changed).toBe(false);
    expect(scope.rust_changed).toBe(false);
    expect(scope.performance_any).toBe(false);
    expect(scope.package_sensitive).toBe(false);
  });

  it("routes React and CSS changes only to frontend checks", () => {
    const scope = route(["src/App.tsx", "src/styles.css"]);
    expect(scope.docs_only).toBe(false);
    expect(scope.frontend_changed).toBe(true);
    expect(scope.rust_changed).toBe(false);
    expect(scope.release_sensitive).toBe(false);
    expect(performanceFlags(scope)).toEqual([false, false, false, false, false, false]);
  });

  it("routes committed real-browser gates and their package contracts to frontend checks", () => {
    for (const changedPath of [
      "scripts/runW2-01BrowserGate.mjs",
      "scripts/w2-01-browser-gate.mjs",
      "scripts/runW2-10BrowserGate.mjs",
      "scripts/runW2-11BrowserGate.mjs",
      "package.json",
      "package-lock.json",
    ]) {
      const scope = route([changedPath]);
      expect(scope.frontend_changed, changedPath).toBe(true);
    }
  });

  it("routes Global Search changes to Search 100k only", () => {
    const scope = route(["src-tauri/src/global_index/query.rs"]);
    expect(performanceFlags(scope)).toEqual([true, false, false, false, false, false]);
    expect(scope.rust_changed).toBe(true);
    expect(scope.macos_sensitive).toBe(true);
  });

  it("does not confuse research-only paths with product Search changes", () => {
    for (const changedPath of [
      "research/zen-decision-bench/src/core.mjs",
      "research/zen-decision-bench/fixtures/pilot.v1.jsonl",
      "tests/zenDecisionBenchResearchContract.test.ts",
    ]) {
      const scope = route([changedPath]);
      expect(scope.perf_search, changedPath).toBe(false);
    }

    expect(route(["src/api/globalSearchApi.ts"]).perf_search).toBe(true);
    expect(route(["src/search.css"]).perf_search).toBe(true);
  });

  it("routes scanner changes to Scan/Schema 100k only", () => {
    const scope = route(["src-tauri/src/scanner/reconcile.rs"]);
    expect(performanceFlags(scope)).toEqual([false, true, false, false, false, false]);
  });

  it("routes managed-scan admission changes to the Native macOS performance lane", () => {
    const scope = route(["src-tauri/src/db/queries/scan.rs"]);
    expect(scope.perf_scan_schema).toBe(true);
    expect(scope.performance_sensitive).toBe(true);
    expect(scope.macos_sensitive).toBe(true);
  });

  it("routes File Library and Content changes to Library/Content 100k only", () => {
    const scope = route(["src-tauri/src/db/queries/library/query.rs"]);
    expect(performanceFlags(scope)).toEqual([false, false, true, false, false, false]);
  });

  it("routes Intelligence changes to the Intelligence 100k suite only", () => {
    const scope = route(["src-tauri/src/db/queries/organization/projection.rs"]);
    expect(performanceFlags(scope)).toEqual([false, false, false, true, false, false]);
  });

  it("routes File Workspace changes to the Workspace Foundation suite only", () => {
    const scope = route(["src-tauri/src/file_workspace/browse/mod.rs"]);
    expect(performanceFlags(scope)).toEqual([false, false, false, false, true, false]);
    expect(scope.performance_sensitive).toBe(true);
  });

  it("routes Preview implementation and Phase A harness changes to Preview Platform", () => {
    for (const [changedPath, workspaceExpected, performanceSensitiveExpected] of [
      ["src-tauri/src/file_workspace/preview.rs", true, true],
      ["src-tauri/src/file_workspace/integration/preview.rs", true, true],
      ["scripts/runW3-10PhaseABrowserHarness.mjs", false, false],
    ] as const) {
      const scope = route([changedPath]);
      expect(scope.perf_preview_platform, changedPath).toBe(true);
      expect(scope.perf_workspace_foundation, changedPath).toBe(workspaceExpected);
      expect(scope.performance_sensitive, changedPath).toBe(performanceSensitiveExpected);
    }
  });

  it("routes the Windows native Preview Handler workspace to its dedicated lane", () => {
    for (const changedPath of [
      "src-tauri/native/windows-preview-handler/src/com.rs",
      "src-tauri/native/windows-preview-handler-harness/src/main.rs",
      "src-tauri/native/preview-representation/src/lib.rs",
    ]) {
      const scope = route([changedPath]);
      expect(scope.windows_native_preview_handler_changed, changedPath).toBe(true);
      expect(scope.rust_changed, changedPath).toBe(true);
    }

    const appPreview = route(["src-tauri/src/file_workspace/preview.rs"]);
    expect(appPreview.windows_native_preview_handler_changed).toBe(false);
  });

  it("routes Global Index and exact-service qualification inputs to the hosted Windows lane only", () => {
    for (const changedPath of [
      "src-tauri/src/global_index/windows/mft.rs",
      "src-tauri/src/global_index/windows/service_host.rs",
      "src-tauri/native-qa/global_index_probe.rs",
      "scripts/qualifyWindowsGlobalIndexService.ps1",
      ".github/workflows/ci.yml",
    ]) {
      expect(
        route([changedPath]).windows_global_index_service_qualification_changed,
        changedPath,
      ).toBe(true);
    }

    for (const changedPath of [
      "src-tauri/src/file_ops.rs",
      "src/App.tsx",
      "docs/project/tasks/ZB-05-NATIVE-GLOBAL-SEARCH-RUNTIME-RESULT.md",
    ]) {
      expect(
        route([changedPath]).windows_global_index_service_qualification_changed,
        changedPath,
      ).toBe(false);
    }
  });

  it("routes DB core and schema changes to every 100k suite without selecting 1M", () => {
    const scope = route(["src-tauri/src/db/schema.rs"]);
    expect(scope.full_validation).toBe(false);
    expect(scope.all_domains_100k).toBe(true);
    expect(performanceFlags(scope)).toEqual([true, true, true, true, true, true]);
    expect(scope.package_sensitive).toBe(false);
  });

  it("routes dependency changes to audit and package smoke without performance suites", () => {
    const scope = route(["package-lock.json"]);
    expect(scope.dependency_sensitive).toBe(true);
    expect(scope.package_sensitive).toBe(true);
    expect(scope.release_sensitive).toBe(true);
    expect(scope.performance_any).toBe(false);
  });

  it("routes both native Cargo workspaces' manifests and lockfiles to dependency audit", () => {
    for (const changedPath of [
      "src-tauri/native/Cargo.toml",
      "src-tauri/native/Cargo.lock",
      "src-tauri/native/host-provided/Cargo.toml",
      "src-tauri/native/preview-representation/Cargo.toml",
      "src-tauri/native/windows-preview-handler/Cargo.toml",
      "src-tauri/native/windows-preview-handler-harness/Cargo.toml",
    ]) {
      const scope = route([changedPath]);
      expect(scope.dependency_sensitive, changedPath).toBe(true);
    }

    const sourceScope = route(["src-tauri/native/windows-preview-handler/src/com.rs"]);
    expect(sourceScope.dependency_sensitive).toBe(false);
  });

  it("routes shared native Preview crates to Preview Platform performance", () => {
    for (const changedPath of [
      "src-tauri/native/host-provided/src/lib.rs",
      "src-tauri/native/preview-representation/src/lib.rs",
      "src-tauri/native/host-provided/Cargo.toml",
      "src-tauri/native/preview-representation/Cargo.toml",
    ]) {
      const scope = route([changedPath]);
      expect(performanceFlags(scope), changedPath).toEqual([false, false, false, false, false, true]);
      expect(scope.perf_preview_platform, changedPath).toBe(true);
      expect(scope.performance_any, changedPath).toBe(true);
      expect(scope.windows_native_preview_handler_changed, changedPath).toBe(true);
      expect(scope.rust_changed, changedPath).toBe(true);
    }
  });

  it("keeps Windows handler-only implementation changes out of Preview Platform performance", () => {
    const scope = route(["src-tauri/native/windows-preview-handler/src/com.rs"]);
    expect(scope.windows_native_preview_handler_changed).toBe(true);
    expect(scope.rust_changed).toBe(true);
    expect(scope.perf_preview_platform).toBe(false);
    expect(scope.performance_any).toBe(false);
  });

  it("routes package and installer inputs to package smoke without 100k performance", () => {
    const scope = route(["src-tauri/tauri.conf.json"]);
    expect(scope.package_sensitive).toBe(true);
    expect(scope.release_sensitive).toBe(true);
    expect(scope.dependency_sensitive).toBe(false);
    expect(scope.performance_any).toBe(false);
  });

  it("routes workflow changes to every 100k suite but keeps Full explicit", () => {
    const scope = route([".github/workflows/ci.yml"]);
    expect(scope.workflow_changed).toBe(true);
    expect(scope.full_validation).toBe(false);
    expect(performanceFlags(scope)).toEqual([true, true, true, true, true, true]);
  });

  it("routes performance harness changes to every 100k suite", () => {
    const scope = route(["scripts/runPerformanceSuite.mjs"]);
    expect(scope.workflow_changed).toBe(true);
    expect(scope.full_validation).toBe(false);
    expect(performanceFlags(scope)).toEqual([true, true, true, true, true, true]);
  });

  it("routes schedule, manual Full, and labeled Full requests to every release-grade gate", () => {
    for (const scope of [
      classifyCiScope({ event: "schedule", changedPaths: [] }),
      classifyCiScope({ event: "workflow_dispatch", changedPaths: [], dispatchFull: true }),
      classifyCiScope({ event: "pull_request", changedPaths: ["src/App.tsx"], prLabels: ["full-validation"] }),
    ]) {
      expect(scope.full_validation).toBe(true);
      expect(scope.all_domains_100k).toBe(true);
      expect(scope.frontend_changed).toBe(true);
      expect(scope.rust_changed).toBe(true);
      expect(scope.windows_native_preview_handler_changed).toBe(true);
      expect(scope.macos_sensitive).toBe(true);
      expect(scope.package_sensitive).toBe(true);
      expect(scope.dependency_sensitive).toBe(true);
      expect(scope.release_sensitive).toBe(true);
      expect(performanceFlags(scope)).toEqual([true, true, true, true, true, true]);
    }
  });

  it("fails closed to all 100k domains when the diff base is missing", () => {
    const scope = route(["src/App.tsx"], { baseMissing: true });
    expect(scope.docs_only).toBe(false);
    expect(scope.full_validation).toBe(false);
    expect(scope.all_domains_100k).toBe(true);
    expect(performanceFlags(scope)).toEqual([true, true, true, true, true, true]);
  });

  it("keeps the two concurrency domains isolated", () => {
    const interactive = readWorkflow(".github/workflows/ci.yml");
    const full = readWorkflow(".github/workflows/ci-full.yml");
    expect(interactive).toContain("ci-interactive-${{ github.ref }}");
    expect(full).toContain("ci-full-${{ github.ref }}");
    expect(interactive).not.toContain("ci-full-${{ github.ref }}");
    expect(full).not.toContain("ci-interactive-${{ github.ref }}");
  });
});
