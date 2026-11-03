import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { FindingRow } from "../src/views/cleanup/FindingRow";
import { isFindingSelectable } from "../src/views/cleanup/cleanupModel";
import { CleanupInspector } from "../src/views/history/HistoryInspector";
import { makeTranslator } from "../src/i18n";
import type { AnalysisFinding, CleanupTrashBatch, CleanupTrashItem, CleanupRestorePreviewItem } from "../src/types/domain";
const t = makeTranslator("en");
const base = { id: "item", batchId: "batch", originalPath: "C:/Downloads/cache", trashPath: "C:/.zen-canvas-trash/cache", name: "cache", size: 10, movedAt: "1700000000000", restoredAt: null, status: "moved", message: "Moved to Fylura Safe Trash and verified file identity." } as CleanupTrashItem;
function render(item: CleanupTrashItem, canRestore: boolean, blockingReason: string | null = null) {
  const batch = { id: "batch", createdAt: item.movedAt, root: "C:/Downloads", totalItems: 1, totalSize: 10, status: "success", items: [item] } as CleanupTrashBatch;
  return renderToStaticMarkup(<CleanupInspector batch={batch} previewById={new Map([[item.id, { ...item, canRestore, blockingReason } as CleanupRestorePreviewItem]])} previewState="ready" selectedIds={new Set()} onToggle={() => undefined} t={t} />);
}
describe("Cleanup History outcome feedback", () => {
  it("never reports a successful move or Restore journal message as a failure", () => {
    expect(render(base, true)).not.toContain(t("restoreErrorGeneric"));
    const restored = render({ ...base, status: "restored", message: "Restored from Fylura Safe Trash." }, false, "already restored");
    expect(restored).not.toContain(t("restoreErrorGeneric"));
    expect(restored).not.toContain(t("restoreErrorAlreadyRestored"));
    expect(restored).toContain(t("historyStatusRestored"));
  });
  it("shows the current backend conflict rather than an old successful-move message", () => {
    const markup = render(base, false, "Original path already exists.");
    expect(markup).toContain(t("restoreErrorTargetExists"));
    expect(markup).not.toContain(t("restoreErrorGeneric"));
  });

});


describe("persisted Cleanup AI advice", () => {
  it("shows the native evidence reason without replacing detector facts or granting execution", () => {
    const finding = { id: "finding", runId: "run", tier: "caution", status: "active", actionKind: "reveal", executable: false, title: "personal files", pathSnapshot: "C:/Downloads/personal", reason: "Deterministic protected facts", sizeBytes: 10, confidence: "estimated", category: "user files", evidenceSummary: { aiAssessment: { schema: "cleanup_ai_assessment_v1", assessment: { candidateId: "finding", reason: "Preserve personal documents; they are not disposable cache.", riskNote: "Original personal work", confidence: 1, trashAllowed: true } } } } as unknown as AnalysisFinding;
    const html = renderToStaticMarkup(<FindingRow finding={finding} selected={false} evidenceExpanded={false} index={0} virtualized={false} interactionLocked={false} t={t} onToggle={() => undefined} onReveal={() => undefined} onToggleEvidence={() => undefined} onRevalidate={() => undefined} tierLabel={() => "Protected"} />);
    expect(html).toContain("Deterministic protected facts");
    expect(html).toContain("Preserve personal documents; they are not disposable cache.");
    expect(html).toContain(t("cleanupSavedAIAdvice"));
    expect(isFindingSelectable(finding)).toBe(false);
    expect(html).not.toContain('aria-pressed="false"');
  });
});
