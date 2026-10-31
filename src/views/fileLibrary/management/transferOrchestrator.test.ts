import { describe, expect, it, vi } from "vitest";
import type { TransferPreview, TransferTicket } from "../../../api/fileManagementApi";
import { orchestrateTransfer } from "./transferOrchestrator";

const tickets: TransferTicket[] = [
  { id: "a", name: "a.txt", path: "C:/source/a.txt", isDirectory: false },
];
const operation = { source_path: "C:/source/a.txt", target_path: "D:/target/a.txt" } as NonNullable<TransferPreview["rows"][number]["preview"]>;
const plan = (rows: TransferPreview["rows"], id = "plan-current"): TransferPreview => ({
  id,
  destinationDirectory: "D:/target",
  rows,
});

function ports(previewResult: TransferPreview, isCurrent = true) {
  return {
    preview: vi.fn(async () => previewResult),
    isCurrent: vi.fn(() => isCurrent),
    beforeExecute: vi.fn(async () => undefined),
    execute: vi.fn(async (accepted: TransferPreview) => ({ previewId: accepted.id })),
  };
}

describe("shared transfer orchestration", () => {
  it("auto-executes the exact fresh preview for a clean intent", async () => {
    const backend = ports(plan([{ ticketId: "a", preview: operation, error: null, skipped: false }]));
    const outcome = await orchestrateTransfer({
      tickets, action: "copy", destinationDirectory: "D:/target", skip: [], keepBoth: [],
    }, backend);

    expect(outcome).toEqual({ kind: "executed", preview: expect.objectContaining({ id: "plan-current" }), result: { previewId: "plan-current" } });
    expect(backend.preview).toHaveBeenCalledOnce();
    expect(backend.beforeExecute).toHaveBeenCalledOnce();
    expect(backend.execute).toHaveBeenCalledWith(expect.objectContaining({ id: "plan-current" }));
  });

  it("pauses on conflicts without executing, then accepts an explicitly skipped fresh preview", async () => {
    const conflict = plan([{ ticketId: "a", preview: null, error: "transfer_target_exists", skipped: false }]);
    const first = ports(conflict);
    const paused = await orchestrateTransfer({
      tickets, action: "copy", destinationDirectory: "D:/target", skip: [], keepBoth: [],
    }, first);
    expect(paused).toEqual({ kind: "conflicts", preview: conflict, ticketIds: ["a"] });
    expect(first.execute).not.toHaveBeenCalled();

    const skipped = plan([{ ticketId: "a", preview: null, error: null, skipped: true }], "all-skipped-preview");
    const second = ports(skipped);
    const resolved = await orchestrateTransfer({
      tickets, action: "copy", destinationDirectory: "D:/target", skip: ["a"], keepBoth: [],
    }, second);
    expect(resolved.kind).toBe("all-skipped");
    expect(second.preview).toHaveBeenCalledOnce();
    expect(second.execute).not.toHaveBeenCalled();
  });

  it("fails closed when the backend does not honor a requested Skip", async () => {
    const backend = ports(plan([{ ticketId: "a", preview: operation, error: null, skipped: false }]));
    const outcome = await orchestrateTransfer({
      tickets, action: "copy", destinationDirectory: "D:/target", skip: ["a"], keepBoth: [],
    }, backend);

    expect(outcome).toMatchObject({ kind: "failed", errors: ["transfer_skip_not_applied"] });
    expect(backend.execute).not.toHaveBeenCalled();
  });

  it("does not execute after a hard error or a stale target context", async () => {
    const failed = ports(plan([{ ticketId: "a", preview: null, error: "transfer_source_changed", skipped: false }]));
    expect((await orchestrateTransfer({
      tickets, action: "copy", destinationDirectory: "D:/target", skip: [], keepBoth: [],
    }, failed)).kind).toBe("failed");
    expect(failed.execute).not.toHaveBeenCalled();

    const stale = ports(plan([{ ticketId: "a", preview: operation, error: null, skipped: false }]), false);
    expect((await orchestrateTransfer({
      tickets, action: "copy", destinationDirectory: "D:/target", skip: [], keepBoth: [],
    }, stale)).kind).toBe("stale");
    expect(stale.execute).not.toHaveBeenCalled();
  });

  it("rechecks target/source context after asynchronous mutation cleanup", async () => {
    const backend = ports(plan([{ ticketId: "a", preview: operation, error: null, skipped: false }]));
    let currentChecks = 0;
    backend.isCurrent.mockImplementation(() => ++currentChecks < 4);
    const outcome = await orchestrateTransfer({
      tickets, action: "copy", destinationDirectory: "D:/target", skip: [], keepBoth: [],
    }, backend);
    expect(outcome.kind).toBe("stale");
    expect(backend.beforeExecute).toHaveBeenCalledOnce();
    expect(backend.execute).not.toHaveBeenCalled();
  });
});
