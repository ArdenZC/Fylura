import { describe, expect, it } from "vitest";
import type { OperationLog } from "../src/types/domain";
import {
  filterHistoryBatches,
  groupOperationLogs,
  isNoOpLog,
  isRestorableLog,
  resolveHistoryBatchExecutionState,
  resolveHistoryBatchState,
  resolveOperationRestoreSelection,
  resolveRestoreExecutionIds,
  restoreIntentMatchesResolution,
  restoreEligibility,
  reconcileRestorableOperationSelection,
  latestUndoBatch,
  selectionForOperationBatch
} from "../src/views/history/historyModel";

function log(id: string, overrides: Partial<OperationLog> = {}): OperationLog {
  return {
    id,
    batch_id: "batch-1",
    operation_type: "move",
    source_path: `C:/before/${id}.txt`,
    target_path: `C:/after/${id}.txt`,
    old_name: `${id}.txt`,
    new_name: `${id}.txt`,
    status: "success",
    error_message: null,
    created_at: "1710000000000",
    can_undo: true,
    path_before: `C:/before/${id}.txt`,
    path_after: `C:/after/${id}.txt`,
    name_before: `${id}.txt`,
    name_after: `${id}.txt`,
    can_restore: true,
    restored_at: null,
    restore_status: "not_restored",
    restore_error: null,
    ...overrides
  };
}

describe("history restore truth model", () => {
  it("uses mutually exclusive reasons matching backend restore gates", () => {
    expect(restoreEligibility(log("ok"))).toEqual({ executable: true, reason: "restorable" });
    expect(restoreEligibility(log("trash", { operation_type: "move_to_trash", path_after: "Recycle Bin" })).reason).toBe("unsupportedOperation");
    expect(restoreEligibility(log("safe-trash", { operation_type: "move_to_trash", path_after: "C:/Zen/.zen-canvas-trash/operations/safe-trash/item.txt" })).reason).toBe("restorable");
    expect(restoreEligibility(log("failed", { status: "failed" })).reason).toBe("failedOperation");
    expect(restoreEligibility(log("pending", { restore_status: "pending" })).reason).toBe("pending");
    expect(restoreEligibility(log("restored", { restore_status: "restored" })).reason).toBe("alreadyRestored");
    expect(restoreEligibility(log("blocked", { can_restore: false })).reason).toBe("backendBlocked");
    expect(restoreEligibility(log("retry", { restore_status: "failed" })).reason).toBe("restoreFailed");
    expect(restoreEligibility(log("review", { status: "manual_review", restore_status: "manual_review" })).reason).toBe("manualReview");
    expect(isRestorableLog(log("retry", { restore_status: "failed" }))).toBe(false);
  });

  it("intersects selected ids with authoritative records and deduplicates", () => {
    const records = [log("ok"), log("blocked", { can_restore: false }), log("skipped", { status: "skipped" })];
    const result = resolveOperationRestoreSelection(records, ["ok", "ok", "blocked", "missing"]);
    expect(result.executableIds).toEqual(["ok"]);
    expect(result.selectedCount).toBe(3);
    expect(result.excludedCount).toBe(2);
    expect(result.missingIds).toEqual(["missing"]);
    expect(result.reasonCounts).toMatchObject({ backendBlocked: 1, unavailable: 1 });
  });

  it("keeps missing batch ids separate and recalculates filtered summaries", () => {
    const batches = groupOperationLogs([log("a", { batch_id: "" }), log("b", { batch_id: "" }), log("c", { batch_id: "batch-2" })]);
    expect(batches).toHaveLength(3);
    const filtered = filterHistoryBatches(batches, "c.txt");
    expect(filtered).toHaveLength(1);
    expect(filtered[0].total).toBe(1);
  });

  it("selects only backend-restorable records and keeps the executable count honest", () => {
    const records = [log("ok"), log("blocked", { can_restore: false })];
    const selected = selectionForOperationBatch(new Set(), records, true);
    expect(selected).toEqual(new Set(["ok"]));
    expect(selectionForOperationBatch(new Set(["ok", "blocked"]), records, false)).toEqual(new Set());
  });

  it("reconciles stale selected ids after the authoritative log changes", () => {
    const selected = new Set(["ok", "blocked", "missing"]);
    const refreshed = [log("ok"), log("blocked", { restore_status: "restored" })];
    expect(reconcileRestorableOperationSelection(refreshed, selected)).toEqual(new Set(["ok"]));
  });

  it("keeps the confirmation whitelist immutable while accepting backend order changes", () => {
    const records = [log("first"), log("second")];
    const initial = resolveOperationRestoreSelection(records, ["first", "second"]);
    const intent = {
      sessionId: "session",
      source: "operation_logs" as const,
      selectedIds: new Set(initial.selectedIds),
      allowedIds: new Set(initial.executableIds),
      selectedCount: initial.selectedCount,
      executableCount: initial.executableCount,
      excludedCount: initial.excludedCount,
      reasonCounts: initial.reasonCounts,
      createdAt: 1,
      revision: 1,
      authorityFingerprint: initial.fingerprint
    };
    const reordered = resolveOperationRestoreSelection([records[1], records[0]], ["first", "second"]);
    expect(restoreIntentMatchesResolution(intent, reordered)).toBe(true);
    expect(resolveRestoreExecutionIds(["second", "forged", "first"], intent, reordered.executableIds)).toEqual(["second", "first"]);
  });

  it("derives a restorable batch state from the aggregate, not the first log", () => {
    expect(resolveHistoryBatchState([log("first"), log("second")])).toBe("restorable");
    expect(resolveHistoryBatchState([log("first"), log("failed", { status: "failed", can_restore: false })])).toBe("partial");
  });

  it("uses only the latest operation batch for one-step undo", () => {
    const older = log("older", { batch_id: "older", created_at: "1710000000000" });
    const latest = log("latest", { batch_id: "latest", created_at: "1710000001000" });
    const latestSecond = log("latest-second", { batch_id: "latest", created_at: "1710000001000", can_restore: false });
    expect(latestUndoBatch([older, latest, latestSecond]).map((item) => item.id)).toEqual(["latest", "latest-second"]);
    const nonRestorable = log("latest-unsafe", { batch_id: "unsafe", created_at: "1710000002000", can_restore: false });
    expect(latestUndoBatch([older, latest, nonRestorable]).map((item) => item.id)).toEqual(["latest-unsafe"]);
  });

  it("keeps pending and explicit no-op states distinct from generic skipped work", () => {
    expect(resolveHistoryBatchExecutionState([log("pending", { status: "pending" })])).toBe("pending");
    expect(resolveHistoryBatchExecutionState([log("pending", { status: "pending" }), log("done")])).toBe("partial");
    const noOp = log("no-op", { operation_type: "no-op" });
    expect(isNoOpLog(noOp)).toBe(true);
    expect(resolveHistoryBatchExecutionState([noOp])).toBe("no_op");
    expect(isRestorableLog(noOp)).toBe(false);
    expect(restoreEligibility(noOp).reason).toBe("noOp");
    expect(resolveHistoryBatchExecutionState([noOp, log("done")])).toBe("partial");
    expect(resolveHistoryBatchExecutionState([log("skipped", { status: "skipped" })])).toBe("skipped");
  });
});
