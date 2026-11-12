import { describe, expect, it } from "vitest";
import { mockInvokeCommand } from "../src/api/browserMockApi";
import { managementFixturePage, managementMock } from "../src/api/fileManagementMock";

describe("browser mock command contract", () => {
  it("throws for commands that were not explicitly registered", async () => {
    await expect(mockInvokeCommand("misspelled_command")).rejects.toThrow(
      "Unsupported mock command: misspelled_command"
    );
  });

  it("simulates Duplicate in place while keeping both files and folders", () => {
    const sourceEntries = {
      case: { name: "Case.txt", path: "C:/U3/Left/Case.txt", isDirectory: false },
      folder: { name: "Subfolder", path: "C:/U3/Left/Subfolder", isDirectory: true },
    };
    const resolve = (ref: { kind: string; fileId?: string }) => {
      if (ref.kind === "managed" && ref.fileId && ref.fileId in sourceEntries) {
        return sourceEntries[ref.fileId as keyof typeof sourceEntries];
      }
      throw Error("fixture_reference_unavailable");
    };
    const tickets = managementMock("file_workspace_transfer_capture", {
      refs: [
        { kind: "managed", fileId: "case" },
        { kind: "managed", fileId: "folder" },
      ],
    }, resolve, () => "C:/U3/Left") as Array<{ id: string }>;
    const preview = managementMock("file_workspace_transfer_preview", {
      ticketIds: tickets.map((ticket) => ticket.id),
      action: "duplicate",
      destinationDirectory: "C:/U3/Left",
      keepBoth: tickets.map((ticket) => ticket.id),
    }, resolve, () => "C:/U3/Left") as { id: string; rows: Array<{ preview: { operation_type: string; target_path: string } }> };

    expect(preview.rows.map((row) => row.preview.operation_type)).toEqual(["duplicate", "duplicate"]);
    expect(preview.rows.map((row) => row.preview.target_path)).toEqual([
      "C:/U3/Left/Case (2).txt",
      "C:/U3/Left/Subfolder (2)",
    ]);
    const result = managementMock("file_workspace_transfer_execute", {
      previewId: preview.id,
      confirmed: true,
    }, resolve, () => "C:/U3/Left") as { logs: Array<{ operation_type: string; can_restore: boolean }> };
    expect(result.logs.map((log) => log.operation_type)).toEqual(["duplicate", "duplicate"]);
    expect(result.logs.some((log) => log.can_restore)).toBe(false);
    const names = managementFixturePage("source", "C:/U3/Left", "duplicate-test")!.map((entry) => entry.name);
    expect(names).toEqual(expect.arrayContaining(["Case.txt", "Case (2).txt", "Subfolder", "Subfolder (2)"]));
  });
});
