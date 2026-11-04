import { describe, expect, it, vi } from "vitest";
import { collectCleanupGroup } from "../src/views/cleanup/collectCleanupGroup";
import { executableCleanupSubset, isFindingSelectable, retainSelectedCleanupFindingsAfterExecution } from "../src/views/cleanup/cleanupModel";
import type { AnalysisFinding, AnalysisRun, OperationPreviewResult } from "../src/types/domain";

const run = { id: "run", revision: 7, safeCount: 205, reviewCount: 0, cautionCount: 0 } as AnalysisRun;
const findings = Array.from({ length: 205 }, (_, index) => ({ id: `finding-${index}`, runId: run.id, tier: "safe", status: "active", executable: true, requiresConfirmation: true, actionKind: "safe_trash_candidate", revision: 2 } as AnalysisFinding));
function api() {
  return {
    getAnalysisRun: vi.fn(async () => run),
    listAnalysisFindings: vi.fn(async ({ cursor }: { cursor?: string | null }) => {
      const offset = Number(cursor ?? 0);
      return { findings: findings.slice(offset, offset + 100), nextCursor: offset + 100 < findings.length ? String(offset + 100) : null, limit: 100, runRevision: run.revision };
    })
  };
}

describe("complete Cleanup groups", () => {
  it("reads all pages, independently of rendered rows, before selecting the group", async () => {
    const current = api(); const progress = vi.fn();
    const all = await collectCleanupGroup(current, run, "safe", new AbortController().signal, progress);
    expect(all).toHaveLength(205);
    expect(current.listAnalysisFindings.mock.calls.map(([request]) => request.cursor)).toEqual([null, "100", "200"]);
    expect(progress.mock.calls.map(([count]) => count)).toEqual([100, 200, 205]);
    expect(all.every(isFindingSelectable)).toBe(true); // Final confirmation is still required, once per batch.
  });
  it("never publishes a partial selection after cancellation", async () => {
    const current = api(); const abort = new AbortController();
    await expect(collectCleanupGroup(current, run, "safe", abort.signal, () => abort.abort())).rejects.toThrow();
    expect(current.listAnalysisFindings).toHaveBeenCalledOnce();
    expect(current.getAnalysisRun).not.toHaveBeenCalled();
  });
  it("rejects incomplete pages and changed revisions", async () => {
    const current = api();
    current.listAnalysisFindings.mockResolvedValueOnce({ findings: findings.slice(0,100), nextCursor: null, limit: 100, runRevision: 7 });
    await expect(collectCleanupGroup(current, run, "safe", new AbortController().signal, () => undefined)).rejects.toThrow("cleanup_group_changed");
    current.listAnalysisFindings.mockResolvedValueOnce({ findings: [], nextCursor: null, limit: 100, runRevision: 8 });
    await expect(collectCleanupGroup(current, run, "safe", new AbortController().signal, () => undefined)).rejects.toThrow("cleanup_group_changed");
  });
  it("rejects page duplication, foreign findings and a run changed at final readback", async () => {
    const current = api();
    current.listAnalysisFindings.mockResolvedValueOnce({ findings: [findings[0], findings[0]], nextCursor: null, limit: 100, runRevision: 7 });
    await expect(collectCleanupGroup(current, run, "safe", new AbortController().signal, () => undefined)).rejects.toThrow("cleanup_group_changed");
    current.listAnalysisFindings.mockResolvedValueOnce({ findings: [{ ...findings[0], runId: "other" }], nextCursor: null, limit: 100, runRevision: 7 });
    await expect(collectCleanupGroup(current, run, "safe", new AbortController().signal, () => undefined)).rejects.toThrow("cleanup_group_changed");
    current.getAnalysisRun.mockResolvedValueOnce({ ...run, revision: 8 });
    await expect(collectCleanupGroup(current, run, "safe", new AbortController().signal, () => undefined)).rejects.toThrow("cleanup_group_changed");
  });
  it("leaves kept and protected items outside execution selection", () => {
    expect(isFindingSelectable({ ...findings[0], decision: "dismissed" })).toBe(false);
    expect(isFindingSelectable({ ...findings[0], tier: "caution" })).toBe(false);
  });
});

describe("qualified Cleanup preview subset", () => {
  const selections = [{ findingId: "one", expectedRevision: 2, reviewConfirmation: { decisionRevision: 3 } }, { findingId: "two", expectedRevision: 4 }];
  const preview = { total: 2, truncated: false, hasMore: false, previews: [{ fileId: "one", status: "pending", is_executable: true }, { fileId: "two", status: "pending", is_executable: false, blocking_reason: "source_changed" }] } as OperationPreviewResult;
  it("retains exact Review confirmation while excluding refused items", () => {
    expect(executableCleanupSubset(preview, selections)).toEqual([selections[0]]);
  });
  it("refuses incomplete, duplicated, foreign or truncated previews", () => {
    for (const bad of [{ ...preview, total: 3 }, { ...preview, truncated: true }, { ...preview, hasMore: true }, { ...preview, previews: [preview.previews[0], preview.previews[0]] }, { ...preview, previews: [{ ...preview.previews[0], fileId: "foreign" }, preview.previews[1]] }]) {
      expect(executableCleanupSubset(bad, selections)).toEqual([]);
    }
  });
});

describe("partial Safe Trash selection reconciliation", () => {
  it("drops successful, changed, protected, missing, and unconfirmed findings while retaining current failures", () => {
    const ids = ["success", "failed", "changed", "protected", "missing", "unconfirmed-review"];
    const refreshed = [
      { ...findings[0], id: "success", revision: 2 },
      { ...findings[1], id: "failed", revision: 5 },
      { ...findings[2], id: "changed", revision: 6, status: "stale" },
      { ...findings[3], id: "protected", revision: 7, tier: "caution", executable: false },
      { ...findings[4], id: "unconfirmed-review", revision: 8, tier: "review", decision: "acknowledged", decisionRevision: null }
    ] as AnalysisFinding[];
    expect(retainSelectedCleanupFindingsAfterExecution(
      ids,
      new Set(["success"]),
      refreshed
    )).toEqual(new Set(["failed"]));
  });
});
