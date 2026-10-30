// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tauriApi } from "../src/api/tauriApi";
import { resetModalInfrastructureForTests } from "../src/components/modal/ModalPortal";
import { makeTranslator } from "../src/i18n";
import type { FileLibrarySummary, OperationPreview } from "../src/types/domain";
import type { LibrarySourceOwner } from "../src/views/fileLibrary/library/librarySourceOwner";
import { BatchRenameAction } from "../src/views/fileLibrary/library/BatchRenameAction";
import { BatchRenameDialog } from "../src/views/fileLibrary/library/BatchRenameDialog";

const t = makeTranslator("en");
const files = [
  { id: "file-a", name: "alpha.txt", isDirectory: false, isStale: false },
  { id: "file-b", name: "beta.txt", isDirectory: false, isStale: false }
] as FileLibrarySummary[];
let container: HTMLDivElement;
let root: Root;

function operationPreview(fileId: string, name: string, options: Partial<OperationPreview> = {}, sourceFiles = files): OperationPreview {
  const file = sourceFiles.find((item) => item.id === fileId)!;
  return {
    id: `preview-${fileId}-${name}`,
    fileId,
    operation_type: "rename",
    source_path: `H:/B1-fixture/${file.name}`,
    target_path: `H:/B1-fixture/${name}`,
    old_name: file.name,
    new_name: name,
    status: "pending",
    risk_level: "Normal",
    confidence: 1,
    requires_confirmation: true,
    reason: "batch rename UI fixture",
    is_executable: true,
    ...options
  };
}

function button(label: string) {
  return [...document.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.includes(label))!;
}

async function clickReview() {
  await act(async () => button(t("libraryBatchRenameReview")).click());
}

async function setTargetName(fileName: string, value: string) {
  const input = [...document.querySelectorAll<HTMLInputElement>('input[aria-label]')]
    .find((item) => item.getAttribute("aria-label")?.endsWith(fileName))!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  resetModalInfrastructureForTests();
  vi.restoreAllMocks();
});

describe("FL-19 batch rename handoff", () => {
  it("requires explicit skip for a blocked row and hands off only executable previews", async () => {
    const good = operationPreview("file-a", "01 alpha.txt");
    const blocked = operationPreview("file-b", "02 beta.txt", { is_executable: false, blocking_reason: "manual_file_target_exists" });
    const request = vi.spyOn(tauriApi, "getManualRenameOperationPreview")
      .mockImplementation(async (fileId, name) => fileId === "file-a" ? good : blocked);
    const onPreview = vi.fn();
    await act(async () => root.render(<BatchRenameDialog files={files} t={t} onClose={vi.fn()} onPreview={onPreview} restoreFocus={() => null}/>));
    await clickReview();
    expect(request).toHaveBeenCalledTimes(2);
    expect(button(t("libraryBatchRenameContinue")).disabled).toBe(true);

    const skip = [...document.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')]
      .find((item) => item.getAttribute("aria-label")?.endsWith("beta.txt"))!;
    await act(async () => skip.click());
    expect(button(t("libraryBatchRenameContinue")).disabled).toBe(false);
    await act(async () => button(t("libraryBatchRenameContinue")).click());
    expect(onPreview).toHaveBeenCalledWith([good]);
    expect(onPreview.mock.calls[0]?.[0]).not.toContain(blocked);
  });

  it("invalidates an edited preview and rejects stale backend names until a fresh preview succeeds", async () => {
    let returnStaleEditedName = true;
    const request = vi.spyOn(tauriApi, "getManualRenameOperationPreview").mockImplementation(async (fileId, name) => {
      if (fileId === "file-a" && name === "renamed-alpha.txt" && returnStaleEditedName) {
        return operationPreview(fileId, "01 alpha.txt");
      }
      return operationPreview(fileId, name);
    });
    const onPreview = vi.fn();
    await act(async () => root.render(<BatchRenameDialog files={files} t={t} onClose={vi.fn()} onPreview={onPreview} restoreFocus={() => null}/>));
    await clickReview();
    expect(button(t("libraryBatchRenameContinue")).disabled).toBe(false);

    await setTargetName("alpha.txt", "renamed-alpha.txt");
    expect(button(t("libraryBatchRenameContinue")).disabled).toBe(true);
    await act(async () => button(t("libraryBatchRenameContinue")).click());
    expect(onPreview).not.toHaveBeenCalled();

    await clickReview();
    expect(request).toHaveBeenCalledWith("file-a", "renamed-alpha.txt");
    expect(button(t("libraryBatchRenameContinue")).disabled).toBe(true);

    returnStaleEditedName = false;
    await clickReview();
    expect(button(t("libraryBatchRenameContinue")).disabled).toBe(false);
    await act(async () => button(t("libraryBatchRenameContinue")).click());
    expect(onPreview).toHaveBeenCalledWith([
      operationPreview("file-a", "renamed-alpha.txt"),
      operationPreview("file-b", "02 beta.txt")
    ]);
    expect(request.mock.calls.filter(([fileId, name]) => fileId === "file-a" && name === "renamed-alpha.txt")).toHaveLength(2);
  });

  it("ignores late preview responses when Escape closes an in-flight batch", async () => {
    const pending = new Map<string, (preview: OperationPreview) => void>();
    vi.spyOn(tauriApi, "getManualRenameOperationPreview").mockImplementation((fileId) => new Promise((resolve) => {
      pending.set(fileId, resolve);
    }));
    const onPreview = vi.fn();
    const onClose = vi.fn();
    function Harness() {
      const [open, setOpen] = useState(true);
      return open ? <BatchRenameDialog files={files} t={t} onClose={() => { onClose(); setOpen(false); }} onPreview={onPreview} restoreFocus={() => null}/> : null;
    }
    await act(async () => root.render(<Harness/>));
    await clickReview();
    expect(pending.size).toBe(2);
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(onClose).toHaveBeenCalledOnce();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => {
      for (const [fileId, resolve] of pending) resolve(operationPreview(fileId, fileId === "file-a" ? "01 alpha.txt" : "02 beta.txt"));
    });
    expect(onPreview).not.toHaveBeenCalled();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it("closes and restores focus when the selected file identities change", async () => {
    const source = (selectedFiles: FileLibrarySummary[]) => ({
      selection: { kind: "explicit" as const, fileIds: selectedFiles.map((file) => file.id) },
      selectedFiles,
      selectedIdList: selectedFiles.map((file) => file.id),
      selectionSummary: null
    }) as unknown as Pick<LibrarySourceOwner, "selection" | "selectedFiles" | "selectedIdList" | "selectionSummary">;
    const onPreview = vi.fn();
    const firstSelection = source(files);
    const nextSelection = source([
      { id: "file-c", name: "gamma.txt", isDirectory: false, isStale: false },
      { id: "file-d", name: "delta.txt", isDirectory: false, isStale: false }
    ] as FileLibrarySummary[]);
    await act(async () => root.render(<BatchRenameAction source={firstSelection} renameAvailable paneVisible t={t} onPreview={onPreview}/>));
    const trigger = button(t("libraryBatchRename"));
    Object.defineProperty(trigger, "getClientRects", { value: () => [{ width: 1, height: 1 }] });
    await act(async () => trigger.focus());
    await act(async () => trigger.click());
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();

    await act(async () => root.render(<BatchRenameAction source={nextSelection} renameAvailable paneVisible t={t} onPreview={onPreview}/>));
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
    expect(document.activeElement).toBe(button(t("libraryBatchRename")));
    expect(onPreview).not.toHaveBeenCalled();
  });

  it("reports progress while preparing previews for a larger loaded selection", async () => {
    const largeFiles = Array.from({ length: 12 }, (_, index) => ({
      id: `large-${index}`,
      name: `file-${index}.txt`,
      isDirectory: false,
      isStale: false
    })) as FileLibrarySummary[];
    const pending: Array<{ fileId: string; name: string; settled: boolean; resolve: (preview: OperationPreview) => void }> = [];
    const request = vi.spyOn(tauriApi, "getManualRenameOperationPreview").mockImplementation((fileId, name) => new Promise((resolve) => {
      pending.push({ fileId, name, settled: false, resolve });
    }));
    await act(async () => root.render(<BatchRenameDialog files={largeFiles} t={t} onClose={vi.fn()} onPreview={vi.fn()} restoreFocus={() => null}/>));
    await clickReview();
    expect(request).toHaveBeenCalledTimes(6);

    let resolved = 0;
    const resolveNext = () => {
      const item = pending.find((candidate) => !candidate.settled);
      if (!item) return false;
      item.settled = true;
      resolved++;
      const index = largeFiles.findIndex((file) => file.id === item.fileId);
      item.resolve(operationPreview(item.fileId, item.name, {}, largeFiles));
      return index >= 0;
    };
    await act(async () => {
      while (resolved < 10) {
        if (!resolveNext()) await Promise.resolve();
        await Promise.resolve();
      }
    });
    expect(document.querySelector('[role="status"]')?.textContent).toContain(`${t("libraryBatchRenameProgress")} 10/12`);

    await act(async () => {
      while (resolved < 12) {
        if (!resolveNext()) await Promise.resolve();
        await Promise.resolve();
      }
    });
    expect(request).toHaveBeenCalledTimes(12);
    expect(button(t("libraryBatchRenameContinue")).disabled).toBe(false);
  });
});
