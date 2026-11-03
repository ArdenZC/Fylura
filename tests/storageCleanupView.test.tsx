import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";
import { StorageCleanupView } from "../src/views/cleanup/StorageCleanupView";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

describe("StorageCleanupView V4.3 durable UX", () => {
  it("starts with an explicit scope chooser and no result fiction", () => {
    const markup = renderToStaticMarkup(<StorageCleanupView t={makeTranslator("zh")} />);
    const source = [
      "src/views/cleanup/StorageCleanupView.tsx",
      "src/views/cleanup/cleanupModel.ts",
      "src/views/cleanup/CleanupGroups.tsx",
      "src/views/cleanup/FindingRow.tsx",
      "src/views/cleanup/useCleanupAnalysisController.ts",
      "src/views/cleanup/useCleanupSelectionController.ts",
      "src/views/cleanup/useCleanupExecutionController.ts"
    ].map(read).join("\n");

    expect(markup).toContain("当前清理范围");
    expect(read("src/i18n/dictionary.ts")).toContain("选择一个磁盘或文件夹开始分析");
    expect(markup).toContain("选择文件夹/磁盘");
    expect(markup).not.toContain("AI 空间清理分析");
    expect(markup).not.toContain("Top 占用排行");
    expect(source).toContain("approvedCleanupPaths");
    expect(source).toContain("startAnalysisRun");
    expect(source).not.toContain("default_scan_roots");
  });

  it("uses Analysis Run and Analysis Finding as the only rendered authority", () => {
    const source = [
      "src/views/cleanup/StorageCleanupView.tsx",
      "src/views/cleanup/cleanupModel.ts",
      "src/views/cleanup/CleanupGroups.tsx",
      "src/views/cleanup/FindingRow.tsx",
      "src/views/cleanup/useCleanupAnalysisController.ts",
      "src/views/cleanup/useCleanupSelectionController.ts",
      "src/views/cleanup/useCleanupExecutionController.ts"
    ].map(read).join("\n");

    expect(source).toContain('data-cleanup-authority="analysis-run-finding"');
    expect(source).toContain("listAnalysisFindings");
    expect(source).toContain("onAnalysisRunUpdated");
    expect(source).toContain("onAnalysisFindingsPublished");
    expect(source).toContain("onAnalysisDetectorUpdated");
    expect(source).not.toContain("useStorageCleanupStore");
    expect(source).not.toContain("StorageAnalysis");
    expect(source).not.toContain("StorageCandidate");
    expect(source).not.toContain("startStorageCleanupScan");
    expect(source).not.toContain("getStorageCleanupCandidatePage");
  });

  it("keeps the finding tiers, backend counts, and caution safety boundary", () => {
    const source = [
      "src/views/cleanup/StorageCleanupView.tsx",
      "src/views/cleanup/cleanupModel.ts",
      "src/views/cleanup/CleanupGroups.tsx",
      "src/views/cleanup/FindingRow.tsx",
      "src/views/cleanup/useCleanupAnalysisController.ts",
      "src/views/cleanup/useCleanupSelectionController.ts",
      "src/views/cleanup/useCleanupExecutionController.ts"
    ].map(read).join("\n");

    expect(source).toContain("run.safeCount");
    expect(source).toContain("run.reviewCount");
    expect(source).toContain("run.cautionCount");
    expect(source).toContain('data-tier={finding.tier}');
    expect(source).toContain("finding.tier === \"caution\"");
    expect(source).toContain("isBackendDefaultSafeFinding");
    expect(source).toContain("finding.decisionRevision");
    expect(source).toContain("reviewConfirmation");
    expect(source).toContain("expectedRevision: finding.revision");
    expect(source).toContain("previewCleanupOperations");
    expect(source).toContain("moveCleanupCandidatesToSafeTrash");
  });

  it("offers one contextual AI recheck action and preserves shared confirmation primitives", () => {
    const source = read("src/views/cleanup/StorageCleanupView.tsx");
    const markup = renderToStaticMarkup(<StorageCleanupView initialRoots={["C:/Users/Zen/Downloads"]} t={makeTranslator("zh")} />);

    expect(source).toContain('t("storageCleanupAIRecheck")');
    expect(source).toContain("analyzeCleanupCandidatesWithAI");
    expect(source).toContain("getAIFeatureReadiness");
    expect(source).toContain("cleanupReadinessText");
    expect(source).not.toContain("getAISettings");
    expect(source).not.toContain("cleanupAiEnabled)");
    expect(source).toContain("ConfirmDialog");
    expect(source).toContain("SideSheet");
    expect(source).not.toContain("AI 分析全部候选");
    expect(source).not.toContain("AI 复查高风险项");
    expect(source).not.toContain("AI 分析已选项");
    expect(markup).toContain("C:/Users/Zen/Downloads");
  });

  it("uses a virtualized, scrollable findings surface for narrow windows", () => {
    const source = read("src/views/cleanup/StorageCleanupView.tsx");

    expect(source).toContain("useVirtualizer");
    expect(source).toContain("max-h-[min(62vh,720px)]");
    expect(source).toContain("overflow-auto");
    expect(source).not.toContain("min-w-[720px]");
    expect(source).not.toContain("grid-cols-[minmax(140px");
  });
});
