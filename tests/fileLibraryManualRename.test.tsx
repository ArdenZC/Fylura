// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { tauriApi } from "../src/api/tauriApi";
import { resetModalInfrastructureForTests } from "../src/components/modal/ModalPortal";
import { makeTranslator } from "../src/i18n";
import type { FileLibrarySummary, OperationPreview } from "../src/types/domain";
import { LibraryContextMenu } from "../src/views/fileLibrary/library/LibraryContextMenu";
import { ManualRenameDialog } from "../src/views/fileLibrary/library/ManualRenameDialog";
import { refreshRenamedSelection } from "../src/store/operationQueue/renameSelectionRefresh";
import { useFileLibraryInspectorStore, useFileLibrarySelectionStore } from "../src/store/useFileLibraryV2Store";
import type { FileLibraryDetail } from "../src/types/domain";

const t = makeTranslator("en");
const file = { id: "owned-file-id", name: "original.txt", isDirectory: false, isStale: false } as FileLibrarySummary;
const preview = { id: "op-manual-rename-owned", fileId: file.id, new_name: "中文.txt", operationFingerprint: "backend-fingerprint", requires_confirmation: true, selected_by_default: false } as OperationPreview;
let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  useFileLibrarySelectionStore.getState().clear();
  useFileLibraryInspectorStore.getState().clear();
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

it("rebinds legacy path IDs after rename and restore using backend-confirmed detail", async () => {
  const oldPath = "F:/owned/original.txt"; const nextPath = "F:/owned/renamed.txt";
  const selected = useFileLibrarySelectionStore.getState();
  selected.setExplicit([oldPath], oldPath, 4);
  useFileLibraryInspectorStore.setState({ selectedId: oldPath });
  const detail = vi.spyOn(tauriApi, "getFileLibraryDetail")
    // A replacement appeared at the original path; never select that object.
    .mockResolvedValueOnce({ id: oldPath, path: oldPath, isStale: false } as FileLibraryDetail)
    .mockResolvedValueOnce({ id: nextPath, path: nextPath, isStale: false } as FileLibraryDetail);
  await refreshRenamedSelection(oldPath, nextPath);
  expect(detail.mock.calls).toEqual([[oldPath], [nextPath]]);
  expect(useFileLibrarySelectionStore.getState()).toMatchObject({ selection: { kind: "explicit", fileIds: [nextPath] }, focusedId: nextPath, anchorIndex: -1 });
  expect(useFileLibraryInspectorStore.getState().selectedId).toBeNull();
  detail.mockRejectedValueOnce(new Error("old ID retired"))
    .mockResolvedValueOnce({ id: oldPath, path: oldPath, isStale: false } as FileLibraryDetail);
  await refreshRenamedSelection(nextPath, oldPath);
  expect(useFileLibrarySelectionStore.getState()).toMatchObject({ selection: { kind: "explicit", fileIds: [oldPath] }, focusedId: oldPath });
});

it("preserves a stable backend ID and does not steal a newer selection during refresh", async () => {
  const store = useFileLibrarySelectionStore.getState();
  store.setExplicit(["stable-id"], "stable-id");
  const detail = vi.spyOn(tauriApi, "getFileLibraryDetail")
    .mockResolvedValueOnce({ id: "stable-id", path: "F:/owned/next.txt", isStale: false } as FileLibraryDetail);
  await refreshRenamedSelection("stable-id", "F:/owned/next.txt");
  expect(useFileLibrarySelectionStore.getState().focusedId).toBe("stable-id");
  let resolve!: (value: FileLibraryDetail) => void;
  detail.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  const pending = refreshRenamedSelection("stable-id", "F:/owned/next.txt");
  store.setExplicit(["newer-user-selection"], "newer-user-selection");
  resolve({ id: "stable-id", path: "F:/owned/next.txt", isStale: false } as FileLibraryDetail);
  await pending;
  expect(useFileLibrarySelectionStore.getState().focusedId).toBe("newer-user-selection");
});

it("clears an obsolete selection when no committed file identity can be resolved", async () => {
  useFileLibrarySelectionStore.getState().setExplicit(["retired-id"], "retired-id", 5);
  vi.spyOn(tauriApi, "getFileLibraryDetail").mockRejectedValue(new Error("unavailable"));
  await refreshRenamedSelection("retired-id", "F:/owned/next.txt");
  expect(useFileLibrarySelectionStore.getState()).toMatchObject({ selection: null, focusedId: "", anchorIndex: -1 });
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  resetModalInfrastructureForTests();
  vi.restoreAllMocks();
});
async function name(value: string) {
  const input = document.querySelector<HTMLInputElement>('input[aria-invalid]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
}
async function submit() {
  await act(async () => { document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
}

it("offers manual rename independently of suggestions and hands an unchanged backend preview to confirmation", async () => {
  const request = vi.spyOn(tauriApi, "getManualRenameOperationPreview").mockResolvedValue(preview);
  const execute = vi.spyOn(tauriApi, "executeMoves");
  const onPreview = vi.fn();
  function Harness() {
    const [rename, setRename] = useState(false);
    return rename ? <ManualRenameDialog file={file} t={t} restoreFocus={() => container} onClose={() => setRename(false)} onPreview={onPreview} />
      : <LibraryContextMenu context={{ file, x: 0, y: 0, restoreFocusElement: null }} t={t} onClose={vi.fn()}
        onPreview={vi.fn()} onReveal={vi.fn()} onOpenContent={vi.fn()} onViewOperations={vi.fn()} onViewSuggestions={vi.fn()}
        onClearSelection={vi.fn()} onRename={() => setRename(true)} />;
  }
  await act(async () => root.render(<Harness />));
  const rename = [...container.querySelectorAll("button")].find((button) => button.textContent === t("libraryManualRename"))!;
  await act(async () => rename.click());
  expect(document.querySelector('[role="dialog"]')).toBeTruthy();
  expect(document.activeElement?.tagName).toBe("INPUT");
  expect(document.body.textContent).toContain(t("libraryRenameDescription"));
  expect(request).not.toHaveBeenCalled();
  await name("中文.txt");
  await submit();
  expect(request).toHaveBeenCalledWith(file.id, "中文.txt");
  expect(onPreview).toHaveBeenCalledWith(preview);
  expect(execute).not.toHaveBeenCalled();
});

it("reports preview errors and permits correction without executing a file action", async () => {
  const request = vi.spyOn(tauriApi, "getManualRenameOperationPreview")
    .mockRejectedValueOnce(new Error("manual_rename_source_read_only")).mockResolvedValueOnce(preview);
  const onPreview = vi.fn();
  await act(async () => root.render(<ManualRenameDialog file={file} t={t} restoreFocus={() => container} onClose={vi.fn()} onPreview={onPreview} />));
  await name("next.txt"); await submit();
  expect(document.querySelector('[role="alert"]')?.textContent).toBe(t("libraryRenameReadOnly"));
  expect(onPreview).not.toHaveBeenCalled();
  await name("中文.txt"); await submit();
  expect(request).toHaveBeenCalledTimes(2);
  expect(onPreview).toHaveBeenCalledOnce();
});

it("cancels a pending preview without accepting its late response or executing", async () => {
  let resolve!: (value: OperationPreview) => void;
  vi.spyOn(tauriApi, "getManualRenameOperationPreview").mockReturnValue(new Promise((done) => { resolve = done; }));
  const execute = vi.spyOn(tauriApi, "executeMoves");
  const onClose = vi.fn(); const onPreview = vi.fn();
  await act(async () => root.render(<ManualRenameDialog file={file} t={t} restoreFocus={() => container} onClose={onClose} onPreview={onPreview} />));
  await name("中文.txt"); await submit();
  const cancel = [...document.querySelectorAll("button")].find((button) => button.textContent === t("cancel"))!;
  await act(async () => cancel.click());
  await act(async () => resolve(preview));
  expect(onClose).toHaveBeenCalledOnce();
  expect(onPreview).not.toHaveBeenCalled();
  expect(execute).not.toHaveBeenCalled();
});

it("does not submit during IME composition and Escape cancels without requesting a preview", async () => {
  const request = vi.spyOn(tauriApi, "getManualRenameOperationPreview");
  const close = vi.fn();
  await act(async () => root.render(<ManualRenameDialog file={file} t={t} restoreFocus={() => container} onClose={close} onPreview={vi.fn()} />));
  const input = document.querySelector("input")!;
  const enter = new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true });
  await act(async () => input.dispatchEvent(enter));
  expect(enter.defaultPrevented).toBe(true);
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(close).toHaveBeenCalledOnce();
  expect(request).not.toHaveBeenCalled();
});
