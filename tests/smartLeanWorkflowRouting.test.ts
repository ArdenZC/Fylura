import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Performance comparisons are opt-in. Rust CI owns routine correctness; this
// workflow keeps controlled writer pairs and master/candidate baselines apart.
const workflow = readFileSync(".github/workflows/smart-lean.yml", "utf8");
const prBlock = workflow.split("\n  push:")[0];
const routes = [...prBlock.matchAll(/^      - '([^']+)'$/gmu)].map((match) => match[1]);
const writerJob = workflow.split("\n  master-candidate-baseline:")[0];
const baselineJob = workflow.split("\n  master-candidate-baseline:")[1]?.split("\n  macos-spotlight-capability:")[0] ?? "";

function triggersFor(file: string): boolean {
  return routes.some((route) => route.endsWith("/**")
    ? file.startsWith(route.slice(0, -2))
    : route === file);
}

describe("Global Index performance workflow routing", () => {
  it("offers explicit labeled performance runs for ingestion and native changes", () => {
    expect(prBlock).toContain("types: [labeled]");
    expect(routes).toEqual([
      "src-tauri/src/global_index/index_scope/**",
      "src-tauri/src/db/schema_index_coverage.rs",
      "src-tauri/src/global_index/repository.rs",
      "src-tauri/src/global_index/coordinator.rs",
      "src-tauri/src/global_index/models.rs",
      "src-tauri/src/global_index/windows/mod.rs",
      "src-tauri/src/global_index/windows/service.rs",
      "src-tauri/src/global_index/macos/mod.rs",
      "src-tauri/src/global_index/windows/mft.rs",
      "src-tauri/src/global_index/windows/usn.rs",
      "src-tauri/src/global_index/windows/fallback.rs",
      "src-tauri/src/global_index/windows/smart_lean_qa.rs",
      "src-tauri/src/global_index/macos/spotlight.rs",
      "src-tauri/src/global_index/macos/fsevents.rs",
      "src-tauri/src/global_index/macos/smart_lean_qa.rs",
      "scripts/qualifySmartLeanMacos.sh",
      ".github/workflows/smart-lean.yml",
    ]);
    for (const file of [
      "src-tauri/src/global_index/index_scope/native.rs",
      "src-tauri/src/global_index/index_scope/repository.rs",
      "src-tauri/src/global_index/index_scope/tests.rs",
      "src-tauri/src/db/schema_index_coverage.rs",
      "src-tauri/src/global_index/repository.rs",
      "src-tauri/src/global_index/coordinator.rs",
      "src-tauri/src/global_index/models.rs",
      "src-tauri/src/global_index/windows/mft.rs",
      "src-tauri/src/global_index/macos/spotlight.rs",
      "scripts/qualifySmartLeanMacos.sh",
    ]) {
      expect(triggersFor(file), file).toBe(true);
    }
  });

  it("keeps pure search, UI and activation edits out of the expensive workflow", () => {
    for (const file of [
      "src-tauri/src/global_index/search.rs",
      "src-tauri/src/global_index/filename_search.rs",
      "src-tauri/src/global_index/commands.rs",
      "src-tauri/src/global_index/windows/volumes.rs",
      "src-tauri/src/global_index/tests/search_source_authorization.rs",
      "src-tauri/src/global_index/tests/filename_search_candidate.rs",
      "src-tauri/src/global_index/tests/global_index_scope_authorization.rs",
      "src-tauri/src/app_control.rs",
      "src/views/settings/SettingsView.tsx",
      "tests/smartLeanWorkflowRouting.test.ts",
    ]) {
      expect(triggersFor(file), file).toBe(false);
    }
    expect(routes).not.toContain("src-tauri/src/global_index/**");
  });

  it("runs the writer pair only on an explicit mode, marked perf push or label", () => {
    expect(writerJob).toContain("inputs.mode == 'writer_pair'");
    expect(writerJob).toContain("[run-global-index-performance]");
    expect(writerJob).toContain("run-global-index-performance");
    expect(writerJob).toContain("ZEN_GLOBAL_WRITER_ENTRIES: '100000'");
    expect(writerJob).toContain("Run 100k writer pair in Release");
    expect(writerJob).toContain(
      "global_index_writer_ingestion_100k_pair -- --ignored --nocapture --test-threads=1",
    );
    expect(writerJob).toContain("Run trigger attribution after writer pair");
    expect(writerJob).toContain(
      "global_index_writer_trigger_contribution_25k -- --ignored --nocapture --test-threads=1",
    );
    expect(writerJob).toContain("cancel-in-progress: false");
    expect(writerJob).not.toContain("github.event_name == 'pull_request' &&");
  });

  it("keeps master/candidate release comparisons manual and preserves bounded sizes", () => {
    expect(workflow).toContain("  workflow_dispatch:");
    expect(workflow).toContain("options: [writer_pair, master_candidate, macos_spotlight, filename_first_pair]");
    expect(workflow).toContain("default: '100000'");
    expect(workflow).toContain("options: ['1000', '100000', '200000']");
    expect(baselineJob).toContain("if: github.event_name == 'workflow_dispatch' && inputs.mode == 'master_candidate'");
    expect(baselineJob).toContain("global_index::coverage::tests");
    expect(baselineJob).toContain("smart_lean_ingestion_cost -- --ignored --nocapture");
    expect(baselineJob).toContain("smart-lean-before.json");
    expect(baselineJob).toContain("smart-lean-after.json");
    expect(baselineJob).toContain("Build Windows native master and candidate binaries");
    expect(baselineJob).toContain("Measure master direct MFT qualification on task-owned NTFS");
    expect(baselineJob).toContain("Measure candidate direct MFT qualification on task-owned NTFS");
    expect(baselineJob).toContain("-DirectOnly");
  });

  it("reports hosted macOS Spotlight capability independently", () => {
    const spotlightJob = workflow.split("\n  macos-spotlight-capability:")[1] ?? "";
    expect(spotlightJob).toContain("inputs.mode == 'macos_spotlight'");
    expect(spotlightJob).toContain("UNVERIFIED");
    expect(spotlightJob).toContain("Synthetic writer results are reported in the separate writer-pair job");
  });
});
