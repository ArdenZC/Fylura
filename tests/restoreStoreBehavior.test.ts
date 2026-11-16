import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OperationLog } from "../src/types/domain";

const api = vi.hoisted(() => ({
  getOperationLogs: vi.fn(),
  getOperationLogsByIds: vi.fn(),
  restoreMoves: vi.fn(),
  cancelOperations: vi.fn()
}));

vi.mock("../src/api/tauriApi", () => ({
  tauriApi: api
}));

import { useOperationQueueStore } from "../src/store/useOperationQueueStore";
import { useFileLibraryStore } from "../src/store/useFileLibraryStore";

function log(id: string, overrides: Partial<OperationLog> = {}): OperationLog {
  return {
    id, batch_id: "batch", operation_type: "move", source_path: `C:/before/${id}`, target_path: `C:/after/${id}`,
    old_name: id, new_name: id, status: "success", error_message: null, created_at: "1710000000000", can_undo: true,
    path_before: `C:/before/${id}`, path_after: `C:/after/${id}`, name_before: id, name_after: id, can_restore: true,
    restored_at: null, restore_status: "not_restored", restore_error: null, ...overrides
  };
}

describe("restore store authoritative selection", () => {
  beforeEach(() => {
    api.getOperationLogs.mockReset();
    api.getOperationLogsByIds.mockReset();
    api.restoreMoves.mockReset();
    api.cancelOperations.mockReset().mockResolvedValue(undefined);
    useFileLibraryStore.setState({ refresh: vi.fn(async () => undefined) });
    useOperationQueueStore.setState({ operationLogs: [], lastRestoreResult: [], restoreError: "", restoreTechnicalError: "", restoreIntent: null, operationProgress: null, activeOperationKind: null });
  });

  it("sends only the authoritative executable intersection", async () => {
    const ok = log("ok");
    const blocked = log("blocked", { can_restore: false });
    api.getOperationLogsByIds.mockResolvedValue([ok, blocked]);
    api.restoreMoves.mockResolvedValue({ logs: [{ ...ok, restore_status: "restored" }], restored: 1, failed: 0 });
    const intent = await useOperationQueueStore.getState().prepareOperationRestoreIntent(["ok", "ok", "blocked", "stale"]);
    expect([...intent!.allowedIds]).toEqual(["ok"]);
    const result = await useOperationQueueStore.getState().confirmOperationRestore(intent!.sessionId);
    expect(api.restoreMoves).toHaveBeenCalledWith([ok]);
    expect(result.status).toBe("executed");
    expect(result.status === "executed" && result.value[0].restore_status).toBe("restored");
    expect(useOperationQueueStore.getState().lastRestoreResult[0].id).toBe("ok");
  });

  it("keeps a failed backend request as a persistent in-page error", async () => {
    const ok = log("ok");
    api.getOperationLogsByIds.mockResolvedValue([ok]);
    api.restoreMoves.mockRejectedValue(new Error("conflict"));
    const intent = await useOperationQueueStore.getState().prepareOperationRestoreIntent(["ok"]);
    expect(await useOperationQueueStore.getState().confirmOperationRestore(intent!.sessionId)).toMatchObject({ status: "rejected" });
    expect(useOperationQueueStore.getState().restoreError).toBeTruthy();
    expect(useOperationQueueStore.getState().restoreTechnicalError).toContain("conflict");
    expect(useOperationQueueStore.getState().operationProgress).toBeNull();
  });
});
