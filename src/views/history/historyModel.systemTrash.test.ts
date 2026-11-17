import { describe, expect, it } from "vitest";
import type { OperationLog } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { isSystemTrashOperation, operationTypeLabel } from "./historyModel";

const identityTranslator = ((key: string) => key) as unknown as Translator;

function operation(operation_type: string, path_after: string): OperationLog {
  return { operation_type, path_after } as OperationLog;
}

describe("System Trash history", () => {
  it("labels system trash separately from Fylura Safe Trash", () => {
    const safeTrash = operation("move_to_trash", "/home/user/.zen-canvas-trash/item.txt");
    const systemTrash = operation("move_to_system_trash", "System Recycle Bin");

    expect(isSystemTrashOperation(safeTrash)).toBe(false);
    expect(operationTypeLabel(safeTrash, identityTranslator)).toBe("operationMoveToTrash");
    expect(isSystemTrashOperation(systemTrash)).toBe(true);
    expect(operationTypeLabel(systemTrash, identityTranslator)).toBe("operationMoveToSystemTrash");
  });

  it("keeps legacy System Recycle Bin journal rows identifiable", () => {
    const legacySystemTrash = operation("move_to_trash", "Recycle Bin");

    expect(isSystemTrashOperation(legacySystemTrash)).toBe(true);
    expect(operationTypeLabel(legacySystemTrash, identityTranslator)).toBe("operationMoveToSystemTrash");
  });
});
