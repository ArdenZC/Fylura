// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fileManagementApi, type SystemClipboardTransfer } from "../src/api/fileManagementApi";
import { readSystemClipboardForPaste, writeSystemClipboardForCopy } from "../src/views/fileLibrary/management/systemClipboardTransfer";
import { useFileManagementStore } from "../src/views/fileLibrary/management/fileManagementStore";

const ticket = { id: "os-file", path: "C:/Source/file.txt", name: "file.txt", isDirectory: false };
const target = () => ({ signature: "a", count: 0, refs: [], routingHint: "C:/Target",
  browse: { sessionId: "a-session", pathRef: { id: "a-root" } }, visible: true,
  refresh: async () => undefined, clear: () => undefined });
let surface = target();
beforeEach(() => {
  surface = target();
  useFileManagementStore.setState({ surfaces: { a: { owner: "a", read: () => surface } }, activePaneId: "a",
    clipboard: null, basket: [], pendingTransfer: null, transferActive: false, transferOpen: false,
    transferTicketIds: [], activeTransferTask: null, activeTransferId: null });
});
afterEach(() => { vi.restoreAllMocks(); useFileManagementStore.setState({ surfaces: {}, clipboard: null }); });
function deferred() {
  let resolve!: (value: SystemClipboardTransfer) => void;
  const promise = new Promise<SystemClipboardTransfer>((done) => { resolve = done; });
  vi.spyOn(fileManagementApi, "systemClipboard").mockReturnValue(promise);
  const release = vi.spyOn(fileManagementApi, "captureTransfer").mockResolvedValue([]);
  return { resolve, release };
}

describe("OS clipboard Paste admission", () => {
  it("admits OS files without an internal clipboard and preserves the OS move intent", async () => {
    const request = deferred();
    const pending = readSystemClipboardForPaste("a");
    request.resolve({ tickets: [ticket], action: "move", owned: false });
    expect(await pending).toMatchObject({ tickets: [ticket], action: "move" });
    expect(fileManagementApi.systemClipboard).toHaveBeenCalledWith({ kind: "read", ticketIds: [] });
    expect(request.release).not.toHaveBeenCalled();
  });
  it.each(["focus", "directory", "closed", "busy"])("releases imported tickets if the target is %s while the OS read waits", async (change) => {
    const request = deferred();
    const pending = readSystemClipboardForPaste("a");
    if (change === "focus") useFileManagementStore.setState({ activePaneId: "b" });
    if (change === "directory") surface.browse.pathRef.id = "different-directory";
    if (change === "closed") useFileManagementStore.setState({ surfaces: {} });
    if (change === "busy") useFileManagementStore.setState({ transferActive: true });
    request.resolve({ tickets: [ticket], action: "copy", owned: false });
    await expect(pending).rejects.toThrow("selection_changed");
    expect(request.release).toHaveBeenCalledWith({ releaseIds: [ticket.id] });
  });
  it("does not release a shared internal clipboard ticket on a stale Paste", async () => {
    useFileManagementStore.setState({ clipboard: { tickets: [ticket], action: "copy" } });
    const request = deferred();
    const pending = readSystemClipboardForPaste("a");
    useFileManagementStore.setState({ activePaneId: "b" });
    request.resolve({ tickets: [ticket], action: "copy", owned: true });
    await expect(pending).rejects.toThrow("selection_changed");
    expect(request.release).not.toHaveBeenCalled();
    expect(useFileManagementStore.getState().clipboard?.tickets).toEqual([ticket]);
  });
  it("orders Copy/Cut and Paste across panes so an older OS write cannot override a newer copy", async () => {
    const a = { ...ticket, id: "copy-a" }, b = { ...ticket, id: "copy-b" };
    let finishA!: (value: SystemClipboardTransfer) => void;
    const firstWrite = new Promise<SystemClipboardTransfer>((resolve) => { finishA = resolve; });
    const operations: string[] = [];
    vi.spyOn(fileManagementApi, "captureTransfer").mockResolvedValue([]);
    vi.spyOn(fileManagementApi, "systemClipboard").mockImplementation(async (request) => {
      operations.push(request.kind === "write" ? request.ticketIds[0] : "paste");
      if (request.kind === "write" && request.ticketIds[0] === a.id) return firstWrite;
      if (request.kind === "write") return { tickets: [], action: "copy", owned: false };
      expect(request.ticketIds).toEqual([b.id]);
      return { tickets: [b], action: "copy", owned: true };
    });
    const first = writeSystemClipboardForCopy(Promise.resolve([a]), true);
    const second = writeSystemClipboardForCopy(Promise.resolve([b]), false);
    const paste = readSystemClipboardForPaste("a");
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(operations).toEqual([a.id]);
    finishA({ tickets: [], action: "move", owned: false });
    await first; await second;
    expect(await paste).toMatchObject({ tickets: [b], action: "copy" });
    expect(operations).toEqual([a.id, b.id, "paste"]);
  });
  it("retains the previous clipboard on write failure and allows the next OS handoff", async () => {
    useFileManagementStore.setState({ clipboard: { tickets: [ticket], action: "copy" } });
    const replacement = { ...ticket, id: "failed-copy" };
    const release = vi.spyOn(fileManagementApi, "captureTransfer").mockResolvedValue([]);
    vi.spyOn(fileManagementApi, "systemClipboard").mockRejectedValueOnce(Error("clipboard_busy"))
      .mockResolvedValueOnce({ tickets: [ticket], action: "copy", owned: true });
    await expect(writeSystemClipboardForCopy(Promise.resolve([replacement]), false)).rejects.toThrow("clipboard_busy");
    expect(release).toHaveBeenCalledWith({ releaseIds: [replacement.id] });
    expect(useFileManagementStore.getState().clipboard?.tickets).toEqual([ticket]);
    expect(await readSystemClipboardForPaste("a")).toMatchObject({ tickets: [ticket], owned: true });
  });

});
