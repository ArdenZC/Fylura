import { describe, expect, it, vi } from "vitest";
import { orchestrateTransfer, type TransferOrchestrationPorts, type TransferOrchestrationRequest } from "../src/views/fileLibrary/management/transferOrchestrator";
import type { TransferPreview } from "../src/api/fileManagementApi";

function request(): TransferOrchestrationRequest {
  return {
    tickets: [],
    action: "create_file",
    destinationDirectory: "/tmp/files",
    newName: "notes.txt",
    skip: [],
    keepBoth: []
  };
}

function ports(preview: TransferPreview): TransferOrchestrationPorts<string> {
  return {
    preview: vi.fn(async () => preview),
    isCurrent: () => true,
    beforeExecute: vi.fn(async () => undefined),
    execute: vi.fn(async () => "completed")
  };
}

describe("default Files empty file creation", () => {
  it("executes only the backend preview for the create-file pseudo row", async () => {
    const preview = { id: "plan-1", destinationDirectory: "/tmp/files", rows: [
      { ticketId: "create-file", preview: {} as never, error: null, skipped: false }
    ] };
    const adapters = ports(preview);
    await expect(orchestrateTransfer(request(), adapters)).resolves.toMatchObject({ kind: "executed", result: "completed" });
    expect(adapters.preview).toHaveBeenCalledWith(expect.objectContaining({ ticketIds: [], action: "create_file", newName: "notes.txt" }));
    expect(adapters.execute).toHaveBeenCalledOnce();
  });

  it("does not execute if the target already exists", async () => {
    const adapters = ports({ id: "plan-2", destinationDirectory: "/tmp/files", rows: [
      { ticketId: "create-file", preview: null, error: "transfer_target_exists", skipped: false }
    ] });
    await expect(orchestrateTransfer(request(), adapters)).resolves.toMatchObject({ kind: "conflicts", ticketIds: ["create-file"] });
    expect(adapters.execute).not.toHaveBeenCalled();
  });
});
