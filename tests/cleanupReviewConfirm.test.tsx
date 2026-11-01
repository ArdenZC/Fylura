// @vitest-environment happy-dom

import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import type { AnalysisFinding, AnalysisRun, OperationPreviewResult } from "../src/types/domain";
import { resetModalInfrastructureForTests } from "../src/components/modal/ModalPortal";
import { StorageCleanupView } from "../src/views/cleanup/StorageCleanupView";

const run: AnalysisRun = {
  id: "analysis-run-review",
  requestKey: "request-review",
  requestAttempt: 1,
  scope: { kind: "approved_cleanup_paths", paths: ["C:/Users/Zen/Downloads"] },
  scopeHash: "scope-review",
  sourceSnapshot: {},
  sourceSnapshotHash: "snapshot-review",
  detectorSet: ["cleanup_heuristics_v1:v1"],
  detectorSetHash: "detectors-review",
  status: "completed",
  phase: "completed",
  revision: 2,
  cancelRequested: false,
  rerunRequired: false,
  detectorsTotal: 1,
  detectorsCompleted: 1,
  detectorsFailed: 0,
  findingsStaged: 1,
  findingsPublished: 1,
  safeCount: 0,
  reviewCount: 1,
  cautionCount: 0,
  exactReclaimableBytes: 0,
  potentialReclaimableBytes: 100,
  warningCount: 0,
  errorCount: 0,
  startedAt: 1,
  finishedAt: 2,
  lastCheckpointAt: 2,
  createdAt: 1,
  updatedAt: 2,
  errorCode: null,
  errorMessage: null
};

function makeFinding(executable: boolean, decision: AnalysisFinding["decision"], revision = 2): AnalysisFinding {
  return {
    id: "finding-review-cache",
    findingKey: "finding-review-cache-key",
    runId: run.id,
    detectorId: "cleanup_heuristics_v1",
    detectorVersion: 1,
    scopeHash: run.scopeHash,
    status: "active",
    tier: "review",
    category: "Downloads",
    actionKind: "safe_trash_candidate",
    title: "review-cache",
    reason: "This folder may contain user-owned files.",
    riskNote: "Review the location before cleanup.",
    confidence: "estimated",
    sizeBytes: 100,
    exactReclaimableBytes: null,
    potentialReclaimableBytes: 100,
    requiresConfirmation: true,
    executable,
    primarySubjectKind: "approved_path",
    primarySubjectId: "C:/Users/Zen/Downloads/review-cache",
    pathSnapshot: "C:/Users/Zen/Downloads/review-cache",
    identitySnapshot: { path: "C:/Users/Zen/Downloads/review-cache" },
    evidenceSummary: { trashAllowed: true },
    revision,
    createdAt: 1,
    updatedAt: revision,
    publishedAt: 1,
    staleAt: null,
    decision,
    snoozedUntil: null,
    decisionRevision: decision === "acknowledged" ? 3 : null
  };
}

describe("cleanup review confirmation", () => {
  let root: Root;
  let currentFinding: AnalysisFinding;
  let previewResult: OperationPreviewResult;
  const nativeGetClientRects = HTMLElement.prototype.getClientRects;
  const nativeGetBoundingClientRect = HTMLElement.prototype.getBoundingClientRect;
  const nativeClientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="app-shell-content"></div><div id="test-root"></div>';
    HTMLElement.prototype.getClientRects = () => [{ width: 120, height: 40, top: 0, left: 0, right: 120, bottom: 40, x: 0, y: 0, toJSON() { return {}; } }] as unknown as DOMRectList;
    HTMLElement.prototype.getBoundingClientRect = () => ({ width: 800, height: 500, top: 0, left: 0, right: 800, bottom: 500, x: 0, y: 0, toJSON() { return {}; } }) as DOMRect;
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, value: 500 });
    currentFinding = makeFinding(false, null);
    previewResult = {
      previews: [{
        id: "preview-review-cache",
        fileId: "finding-review-cache",
        operation_type: "move_to_trash",
        source_path: "C:/Users/Zen/Downloads/review-cache",
        target_path: ".zen-canvas-trash/review-cache",
        old_name: "review-cache",
        new_name: "review-cache",
        status: "pending",
        risk_level: "Caution",
        confidence: 0.8,
        requires_confirmation: true,
        is_executable: true,
        reason: "Review confirmed"
      }],
      total: 1,
      limit: 100,
      offset: 0,
      truncated: false,
      hasMore: false
    };
    root = createRoot(document.getElementById("test-root")!);
  });

  afterEach(() => {
    act(() => root.unmount());
    resetModalInfrastructureForTests();
    HTMLElement.prototype.getClientRects = nativeGetClientRects;
    HTMLElement.prototype.getBoundingClientRect = nativeGetBoundingClientRect;
    if (nativeClientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", nativeClientHeight);
    else delete (HTMLElement.prototype as { clientHeight?: number }).clientHeight;
    document.body.innerHTML = "";
  });

  it("requires acknowledgement, then uses Preview before Safe Trash", async () => {
    const setDecision = vi.fn(async () => {
      currentFinding = makeFinding(true, "acknowledged", 3);
      return {
        findingKey: currentFinding.findingKey,
        decision: "acknowledged" as const,
        snoozedUntil: null,
        note: null,
        revision: 3,
        createdAt: 1,
        updatedAt: 3
      };
    });
    const previewCleanupOperations = vi.fn(async () => previewResult);
    const moveCleanupCandidatesToSafeTrash = vi.fn(async () => ({ moved: 1, skipped: 0, failed: 0, logs: [] }));
    const api = {
      listAnalysisDetectors: async () => [],
      startAnalysisRun: async () => run,
      getActiveAnalysisRun: async () => null,
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      listAnalysisRunDetectors: async () => [{ runId: run.id, detectorId: "cleanup_heuristics_v1", detectorVersion: 1, status: "completed", revision: 1, scannedSubjects: 1, findingsStaged: 1, findingsPublished: 1, exactReclaimableBytes: 0, potentialReclaimableBytes: 100, startedAt: 1, finishedAt: 2, errorCode: null, errorMessage: null }],
      listAnalysisFindings: async ({ tier }: { tier?: string }) => ({ findings: tier === "review" ? [currentFinding] : [], nextCursor: null, limit: 100 }),
      getAnalysisFinding: async () => currentFinding,
      listAnalysisFindingEvidence: async () => [],
      setAnalysisFindingDecision: setDecision,
      revalidateAnalysisFinding: async () => currentFinding,
      previewCleanupOperations,
      moveCleanupCandidatesToSafeTrash,
      revealInFolder: async () => undefined,
      onAnalysisRunUpdated: async () => () => undefined,
      onAnalysisFindingsPublished: async () => () => undefined,
      onAnalysisDetectorUpdated: async () => () => undefined
    };

    await act(async () => {
      root.render(createElement(StorageCleanupView as unknown as (props: Record<string, unknown>) => ReactElement, { api, t: makeTranslator("zh") }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    const reviewTab = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("需人工判断"));
    expect(reviewTab).toBeTruthy();
    await act(async () => reviewTab?.click());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    const acknowledge = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("确认后加入清单"));
    expect(acknowledge).toBeTruthy();
    const findingRow = document.querySelector<HTMLElement>('[data-analysis-finding-id="finding-review-cache"]');
    const rowActions = findingRow?.querySelector<HTMLElement>("[data-cleanup-row-actions]");
    const findingText = findingRow?.textContent ?? "";
    expect(rowActions).toBeTruthy();
    expect(findingText.indexOf("确认后加入清单")).toBeGreaterThanOrEqual(0);
    expect(findingText.indexOf("确认后加入清单")).toBeLessThan(findingText.indexOf("原因"));
    await act(async () => acknowledge?.click());
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("确认加入清理清单");

    const confirmReview = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("确认并加入清单"));
    await act(async () => confirmReview?.click());
    expect(setDecision).toHaveBeenCalledOnce();

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    // The primary row action is a preview; its button text is also its accessible name.
    const previewAction = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("预览所选 Safe Trash 操作"));
    expect(previewAction).toBeTruthy();
    await act(async () => previewAction?.click());
    expect(previewCleanupOperations).toHaveBeenCalledWith(run.id, [{ findingId: currentFinding.id, expectedRevision: currentFinding.revision, reviewConfirmation: { decisionRevision: 3 } }]);
    expect(moveCleanupCandidatesToSafeTrash).not.toHaveBeenCalled();

    const previewConfirm = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("确认进入 Safe Trash"));
    expect(previewConfirm).toBeTruthy();
    await act(async () => previewConfirm?.click());
    expect(moveCleanupCandidatesToSafeTrash).not.toHaveBeenCalled();
    const trashConfirm = [...document.querySelector('[role="alertdialog"]')!.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "移到安全回收站");
    await act(async () => trashConfirm?.click());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(moveCleanupCandidatesToSafeTrash).toHaveBeenCalledOnce();
  });
});
