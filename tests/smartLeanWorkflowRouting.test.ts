import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// The high-cost 100k Smart Lean matrix is for coverage admission/ingestion,
// not every edit under global_index (such as search, query or result activation).
const workflow = readFileSync(".github/workflows/smart-lean.yml", "utf8");
const prBlock = workflow.split("\n  workflow_dispatch:")[0];
const routes = [...prBlock.matchAll(/^      - '([^']+)'$/gmu)].map((match) => match[1]);

function triggersFor(file: string): boolean {
  return routes.some((route) => route.endsWith("/**")
    ? file.startsWith(route.slice(0, -2))
    : route === file);
}

describe("Smart Lean expensive PR qualification routing", () => {
  it("runs for coverage policy, source ingestion and native qualification changes", () => {
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
    ]);
    for (const file of [
      "src-tauri/src/global_index/index_scope/native.rs",
      "src-tauri/src/global_index/index_scope/repository.rs",
      "src-tauri/src/global_index/index_scope/tests.rs",
      "src-tauri/src/db/schema_index_coverage.rs",
      "src-tauri/src/global_index/repository.rs",
      "src-tauri/src/global_index/coordinator.rs",
      "src-tauri/src/global_index/models.rs",
      "src-tauri/src/global_index/windows/mod.rs",
      "src-tauri/src/global_index/windows/service.rs",
      "src-tauri/src/global_index/macos/mod.rs",
      "src-tauri/src/global_index/windows/mft.rs",
      "src-tauri/src/global_index/windows/smart_lean_qa.rs",
      "src-tauri/src/global_index/macos/spotlight.rs",
      "scripts/qualifySmartLeanMacos.sh",
    ]) {
      expect(triggersFor(file), file).toBe(true);
    }
  });

  it("does not launch 100k ingestion for pure query, UI and activation edits", () => {
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
      ".github/workflows/smart-lean.yml",
      "tests/smartLeanWorkflowRouting.test.ts",
    ]) {
      expect(triggersFor(file), file).toBe(false);
    }
    expect(routes).not.toContain("src-tauri/src/global_index/**");
  });

  it("triggers the owner check when an ingestion edit is mixed with query changes", () => {
    // A PR that also touches repository.rs or coordinator.rs is not query-only,
    // even if many of its changes are to SQL ranking or Settings.
    const mixedPaths = [
      "src-tauri/src/global_index/search.rs",
      "src-tauri/src/global_index/repository.rs",
      "src-tauri/src/global_index/coordinator.rs",
      "src/views/settings/SettingsView.tsx",
    ];
    expect(mixedPaths.some(triggersFor)).toBe(true);
    expect(["src-tauri/src/global_index/search.rs", "src/views/settings/SettingsView.tsx"].some(triggersFor)).toBe(false);
  });

  it("keeps manual 1k/100k/200k dispatch and the full coverage policy gate", () => {
    expect(workflow).toContain("  workflow_dispatch:");
    expect(workflow).toContain("default: '100000'");
    expect(workflow).toContain("options: ['1000', '100000', '200000']");
    expect(workflow).toContain("global_index::coverage::tests");
    expect(workflow).toContain("scripts/qualifySmartLeanMacos.sh");
    expect(workflow).toContain("smart_lean_ingestion_cost -- --ignored --nocapture");
    expect(workflow).toContain("cancel-in-progress: true");
  });
});
