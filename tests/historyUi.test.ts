import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

describe("history refinement UI contracts", () => {
  it("keeps the safety message neutral and long paths readable", () => {
    const restore = read("src/views/restore/RestoreView.tsx");
    const batchList = read("src/views/history/HistoryBatchList.tsx");
    const inspector = read("src/views/history/HistoryInspector.tsx");
    const appShell = read("src/components/AppShell.tsx");
    const zh = makeTranslator("zh");

    expect(appShell).toContain('return t("viewDescRestore")');
    expect(zh("viewDescRestore")).toBe("只恢复 Fylura 执行过的操作");
    expect(restore).toContain("primaryFilterButtons");
    expect(restore).toContain("moreFilterButtons");
    expect(restore).toContain('t("historyMoreFilters")');
    expect(restore).toContain('role="dialog"');
    expect(restore).toContain("moreFiltersTriggerRef");
    expect(batchList).not.toContain("break-all");
    expect(batchList).toContain("line-clamp-2");
    expect(batchList).toContain("compactPath");
    expect(batchList).toContain("title={first?.path_after || first?.target_path}");
    expect(batchList).toContain("operationTypeLabel(first, t)");
    expect(inspector).not.toContain("break-all");
    expect(inspector).toContain("break-words font-mono");
    expect(inspector).toContain("title={title ?? value}");
    expect(inspector).toContain('t("historyRestoreDestination")');
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
    expect(batchList).toContain("batch.logs.filter(isRestorableLog)");
  });
});
