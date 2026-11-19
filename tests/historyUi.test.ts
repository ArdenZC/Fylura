import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

describe("history refinement UI contracts", () => {
  it("keeps the safety message neutral and long paths readable", () => {
    const restore = read("src/views/restore/RestoreView.tsx");
    const batchList = read("src/views/history/HistoryBatchList.tsx");
    const inspector = read("src/views/history/HistoryInspector.tsx");

    expect(restore).toContain("zc-neutral-soft");
    expect(restore).toContain('t("historySafetyBoundary")');
    expect(restore).toContain("primaryFilterButtons");
    expect(restore).toContain("moreFilterButtons");
    expect(restore).toContain('t("historyMoreFilters")');
    expect(restore).toContain('role="dialog"');
    expect(restore).toContain("moreFiltersTriggerRef");
    expect(batchList).not.toContain("break-all");
    expect(batchList).toContain("line-clamp-2");
    expect(batchList).toContain("compactPath");
    expect(batchList).toContain("formatDisplayPath(sourcePath)} → ${formatDisplayPath(destinationPath)");
    expect(batchList).toContain("historyOperationSource");
    expect(inspector).not.toContain("break-all");
    expect(inspector).toContain("break-words font-mono");
    expect(inspector).toContain("title={title ?? value}");
    expect(inspector).toContain('t("historyPathChange")');
    expect(batchList).toContain('t("historyViewDetails")');
    expect(inspector).toContain('t("historyRestoreOriginalUnavailable")');
  });

  it("keeps history summaries scannable without changing restore eligibility", () => {
    const restore = read("src/views/restore/RestoreView.tsx");
    const batchList = read("src/views/history/HistoryBatchList.tsx");
    const inspector = read("src/views/history/HistoryInspector.tsx");
    const model = read("src/views/history/historyModel.ts");

    expect(inspector).toContain('t("historySummaryOperations")');
    expect(inspector).toContain('t("historySummaryRestorable")');
    expect(inspector).toContain("grid-cols-2 gap-2 text-xs sm:grid-cols-4");
    expect(model).toContain("resolveOperationRestoreSelection");
    expect(model).toContain("isRestorableLog");
    expect(model).toContain("isNoOpLog");
    expect(model).toContain("reconcileRestorableOperationSelection");
    expect(restore).toContain("reconcileRestorableOperationSelection");
    expect(inspector).toContain("disabled={!eligible}");
    expect(inspector).toContain("operationItemStatusLabel(log, t)");
    expect(batchList).toContain("batch.logs.filter(isRestorableLog)");
  });
});
