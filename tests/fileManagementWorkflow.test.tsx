// @vitest-environment happy-dom
import { act, useState } from "react";
import { PaneContext, type PaneContextValue } from "../src/views/fileLibrary/workbench/PaneContext";
import { defaultFileLibraryProjection } from "../src/store/useFileLibraryV2Store";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import { FileManagementToolbar } from "../src/views/fileLibrary/management/FileManagementToolbar";
import { useFileManagementStore } from "../src/views/fileLibrary/management/fileManagementStore";
import { useOperationQueueStore } from "../src/store/useOperationQueueStore";
import { fileManagementApi as api, type PersistentBasketEntry, type TransferTicket, type TransferPreview } from "../src/api/fileManagementApi";
import { tauriApi } from "../src/api/tauriApi";
import type { OperationLog, OperationPreview } from "../src/types/domain";
const mocks = vi.hoisted(() => ({ close: vi.fn(), navigate: vi.fn(), setView: vi.fn(), openDirectory: vi.fn(async () => "C:/Target"), platform: "win32", defaultDeleteDestination: "fylura_trash", refreshHistory: vi.fn(async () => []), cancelOperations: vi.fn(async () => undefined) }));
vi.mock("../src/contexts/AppContexts", async () => {
  const { makeTranslator } = await import("../src/i18n"); const t = makeTranslator("en");
  return { useI18nContext: () => ({ t, language: "en" }), useChromeContext: () => ({ platform: mocks.platform }), useNavigationContext: () => ({ setView: mocks.setView }), useSettingsContext: () => ({ settings: { deleteDestination: mocks.defaultDeleteDestination } }) };
});
vi.mock("../src/views/fileLibrary/FileLibraryExperienceProvider", () => ({ useFileLibraryExperience: () => ({ controller: { openBrowse: mocks.navigate } }) }));
vi.mock("../src/views/fileLibrary/preview/PreviewExperienceProvider", () => ({ usePreviewExperience: () => ({ controller: { close: mocks.close, closeForFileOperation: mocks.close } }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.openDirectory, save: vi.fn() }));
vi.mock("../src/store/useOperationQueueStore", () => {
  const state = { operationLogs: [], operationProgress: null, activeOperationKind: null, isOperationCanceling: false } as Record<string, any>;
  const useOperationQueueStore = Object.assign((selector: (value: typeof state) => unknown) => selector(state), {
    getState: () => ({ ...state, refreshOperationLogs: mocks.refreshHistory, cancelOperations: async () => { state.isOperationCanceling = true; await mocks.cancelOperations(); } }),
    setState: (update: Record<string, any>) => Object.assign(state, update),
  });
  return { useOperationQueueStore };
});
const t = makeTranslator("en");
const tickets: TransferTicket[] = [{id:"one",name:"one.txt",path:"C:/Source/one.txt",isDirectory:false},{id:"two",name:"two.txt",path:"C:/Other/two.txt",isDirectory:false}];
const basketEntries: PersistentBasketEntry[] = [{ id: "basket-one", name: "one.txt", path: "C:/Source/one.txt", isDirectory: false, status: "available" }, { id: "basket-two", name: "two.txt", path: "C:/Other/two.txt", isDirectory: false, status: "available" }];
const folderTicket: TransferTicket = { id: "folder", name: "Folder", path: "C:/Source/Folder", isDirectory: true };
function plan(): TransferPreview { return { id: "authority-preview", destinationDirectory: "C:/Target", rows: tickets.map((ticket) => ({ticketId: ticket.id, error: null, skipped: false, preview: { id: `op-${ticket.id}`, fileId: ticket.id, operation_type:"copy",source_path:ticket.path,target_path:`C:/Target/${ticket.name}`,old_name:ticket.name,new_name:ticket.name,status:"pending",risk_level:"Normal",confidence:1,requires_confirmation:true,suggested_action:"Copy",reason:"manual",operationFingerprint:"bound-proof"} })) }; }
function actionPlan(ticket: TransferTicket, action: string, destination: string, name: string): TransferPreview {
  const createDirectory = action === "create_directory";
  const trash = action === "trash";
  const target = createDirectory ? `${destination}/${name}` : trash
    ? `${ticket.path.slice(0, ticket.path.lastIndexOf("/"))}/.zen-canvas-trash/operations/fixture/${ticket.name}`
    : `${destination}/${name}`;
  return { id: `authority-${action}`, destinationDirectory: destination, rows: [{
    ticketId: createDirectory ? "create-directory" : ticket.id,
    error: null,
    skipped: false,
    preview: {
      id: `op-${action}`, fileId: ticket.id, operation_type: (trash ? "move_to_trash" : action) as OperationPreview["operation_type"],
      source_path: createDirectory ? destination : ticket.path, target_path: target,
      old_name: createDirectory ? name : ticket.name, new_name: name, status: "pending", risk_level: "Normal",
      confidence: 1, requires_confirmation: true, suggested_action: action === "rename" ? "Rename" : trash ? "Move" : "Copy", reason: "manual",
      operationFingerprint: `bound-${action}`
    }
  }] };
}
function folderPlan(action: "copy" | "move", destinationDirectory = "C:/Target"): TransferPreview { return { id: `folder-${action}-preview`, destinationDirectory, rows: [{ ticketId: folderTicket.id, error: null, skipped: false, preview: { id: `folder-${action}`, fileId: folderTicket.id, operation_type: action, source_path: folderTicket.path, target_path: `${destinationDirectory}/${folderTicket.name}`, old_name: folderTicket.name, new_name: folderTicket.name, status: "pending", risk_level: "Normal", confidence: 1, requires_confirmation: true, suggested_action: action === "move" ? "Move" : "Copy", reason: "manual", operationFingerprint: `folder-${action}-fingerprint` } as OperationPreview }] }; }
function result(statuses: string[]): OperationLog[] { return tickets.map((ticket,index) => ({ id:`log-${ticket.id}`,batch_id:"batch",operation_type:"move",source_path:ticket.path,target_path:`C:/Target/${ticket.name}`,old_name:ticket.name,new_name:ticket.name,status:statuses[index],error_message:statuses[index]==="failed"?"source_locked":null,created_at:"2026-10-26",can_undo:false,path_before:ticket.path,path_after:`C:/Target/${ticket.name}`,name_before:ticket.name,name_after:ticket.name,can_restore:false,restored_at:null,restore_status:"not_restored",restore_error:null } as OperationLog)); }
let root: Root; let host: HTMLDivElement;
const refresh = vi.fn(async () => undefined); const clear = vi.fn();
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === text || b.textContent?.trim().startsWith(text))!;
async function settle() { await act(async () => { await new Promise((resolve) => setTimeout(resolve, 5)); }); }
async function click(text: string) { await act(async () => { button(text).click(); }); await settle(); }
async function fill(input: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function render() { await act(async () => { root.render(<FileManagementToolbar/>); }); await settle(); }
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  document.body.innerHTML = '<main id="app-shell-content"></main>'; host = document.createElement("div"); document.body.querySelector("main")!.appendChild(host); root = createRoot(host);
  vi.spyOn(api,"captureTransfer").mockResolvedValue(tickets);
  vi.spyOn(api,"captureTransferSelectionPage").mockResolvedValue([]);
  vi.spyOn(api,"basketPage").mockResolvedValue({ entries: [], total: 0 });
  vi.spyOn(api,"basketAdd").mockResolvedValue(2);
  vi.spyOn(api,"basketCapture").mockResolvedValue(tickets);
  vi.spyOn(api,"basketRemove").mockResolvedValue(1);
  vi.spyOn(api,"basketClear").mockResolvedValue(0);
  vi.spyOn(api,"basketReconcileMoves").mockResolvedValue(0);
  vi.spyOn(api,"fileLibraryPlaces").mockResolvedValue([{id:"place",name:"Target",routingHint:"C:/Target",favorite:true,current:true,visitedAt:1,unavailable:false}]);
  vi.spyOn(api,"previewTransfer").mockResolvedValue(plan());
  vi.spyOn(api,"executeTransfer").mockResolvedValue({batch_id:"batch",logs:result(["success","success"])});
  vi.spyOn(api,"createZip").mockResolvedValue({ batch_id: "zip-batch", logs: [{ ...result(["success"])[0]!, operation_type: "zip_create", can_undo: false, can_restore: false } as OperationLog] });
  vi.spyOn(api,"extractZip").mockResolvedValue({ batch_id: "unzip-batch", logs: [{ ...result(["success"])[0]!, operation_type: "zip_extract", can_undo: false, can_restore: false } as OperationLog] });
  vi.spyOn(tauriApi,"onOperationProgress").mockResolvedValue(() => undefined);
  vi.spyOn(tauriApi,"cancelOperations").mockResolvedValue();
  useFileManagementStore.setState({owner:"owner",surface:()=>({signature:"selected",count:2,selection:{kind:"explicit",fileIds:["managed-one","managed-two"]},browse:{sessionId:"session",pathRef:{id:"root"}},refresh,clear}),basket:[],basketTotal:0,clipboard:null,results:[],statuses:{},basketOpen:false,pendingAction:null,surfaces:{},activePaneId:null,pendingTransfer:null,transferOpen:false,transferActive:false,transferRunning:false,activeTransferId:null,activeTransferTask:null,transferTicketIds:[],externalDropAction:"copy"});
  useOperationQueueStore.setState({ operationLogs: [], operationProgress: null, activeOperationKind: null, isOperationCanceling: false });
  mocks.platform = "win32"; mocks.navigate.mockReset().mockResolvedValue({}); mocks.setView.mockClear();
  mocks.openDirectory.mockReset().mockResolvedValue("C:/Target");
  mocks.defaultDeleteDestination = "fylura_trash";
  refresh.mockClear();clear.mockClear();mocks.close.mockClear();mocks.refreshHistory.mockClear();mocks.cancelOperations.mockClear();
});
afterEach(async () => { await act(async () => root.unmount()); vi.restoreAllMocks(); document.body.innerHTML=""; });
describe("File Library manual management workflow", () => {
  it("copies a large directory selection to a chosen destination in bounded batches", async () => {
    const allTickets: TransferTicket[] = Array.from({ length: 1001 }, (_, index) => ({
      id: `selected-${index}`, name: `file-${index}.txt`, path: `C:/Source/file-${index}.txt`, isDirectory: false
    }));
    const source = {
      signature: "directory-selected", count: allTickets.length,
      directorySelection: {
        snapshotId: "directory-snapshot", count: allTickets.length, fileCount: allTickets.length, directoryCount: 0,
        sessionId: "browse-session", pathRefId: "source-root", queryKey: JSON.stringify({ text: null, entryKind: "all" }),
        includeHidden: true, revision: 0, excludedEntryIds: new Set<string>()
      },
      browse: { sessionId: "browse-session", pathRef: { id: "source-root" } }, routingHint: "C:/Source", refresh, clear
    };
    useFileManagementStore.setState({ surface: () => source });
    vi.mocked(api.captureTransferSelectionPage).mockImplementation(async ({ offset }) => allTickets.slice(offset, offset + 1000));
    let batchTickets: TransferTicket[] = [];
    vi.mocked(api.previewTransfer).mockImplementation(async (request) => {
      batchTickets = request.ticketIds.map((id) => allTickets.find((ticket) => ticket.id === id)!);
      return {
        id: `copy-preview-${request.ticketIds[0]}`, destinationDirectory: request.destinationDirectory,
        rows: batchTickets.map((ticket) => ({
          ticketId: ticket.id, error: null, skipped: false,
          preview: {
            id: `op-${ticket.id}`, fileId: ticket.id, operation_type: "copy", source_path: ticket.path,
            target_path: `${request.destinationDirectory}/${ticket.name}`, old_name: ticket.name, new_name: ticket.name,
            status: "pending", risk_level: "Normal", confidence: 1, requires_confirmation: true,
            suggested_action: "Copy", reason: "manual", operationFingerprint: `fingerprint-${ticket.id}`
          } as OperationPreview
        }))
      };
    });
    vi.mocked(api.executeTransfer).mockImplementation(async () => ({
      batch_id: `copy-batch-${batchTickets.length}`,
      logs: batchTickets.map((ticket) => ({
        id: `log-${ticket.id}`, batch_id: `copy-batch-${batchTickets.length}`, operation_type: "copy",
        source_path: ticket.path, target_path: `C:/Target/${ticket.name}`, old_name: ticket.name, new_name: ticket.name,
        status: "success", error_message: null, created_at: "2026-11-13", can_undo: false,
        path_before: ticket.path, path_after: `C:/Target/${ticket.name}`, name_before: ticket.name,
        name_after: ticket.name, can_restore: false, restored_at: null, restore_status: "not_restored", restore_error: null
      } as OperationLog))
    }));

    await render();
    await click(t("fmCopy"));
    for (let attempt = 0; attempt < 20 && vi.mocked(api.executeTransfer).mock.calls.length < 2; attempt += 1) await settle();

    expect(mocks.openDirectory).toHaveBeenCalledWith({ directory: true, multiple: false, title: t("fmDestination") });
    expect(api.captureTransferSelectionPage).toHaveBeenNthCalledWith(2, {
      sessionId: "browse-session", snapshotId: "directory-snapshot", offset: 1000, excludedEntryIds: [], advanceSnapshot: true
    });
    expect(api.executeTransfer).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).toContain("processed 1001/1001");
  });

  it("processes a large directory delete as identity-bound 1,000 item batches", async () => {
    const allTickets = Array.from({ length: 1001 }, (_, index) => ({
      id: `selected-${index}`,
      name: `file-${index}.txt`,
      path: `C:/Source/file-${index}.txt`,
      isDirectory: false
    }));
    const source = {
      signature: "directory-selected",
      count: allTickets.length,
      directorySelection: {
        snapshotId: "directory-snapshot",
        count: allTickets.length,
        fileCount: allTickets.length,
        directoryCount: 0,
        sessionId: "browse-session",
        pathRefId: "source-root",
        queryKey: JSON.stringify({ text: null, entryKind: "all" }),
        includeHidden: true,
        revision: 0,
        excludedEntryIds: new Set<string>()
      },
      browse: { sessionId: "browse-session", pathRef: { id: "source-root" } },
      routingHint: "C:/Source",
      refresh,
      clear
    };
    useFileManagementStore.setState({ surface: () => source });
    vi.mocked(api.captureTransferSelectionPage).mockImplementation(async ({ offset }) => allTickets.slice(offset, offset + 1000));
    let batchTickets: TransferTicket[] = [];
    vi.mocked(api.previewTransfer).mockImplementation(async (request) => {
      batchTickets = request.ticketIds.map((id) => allTickets.find((ticket) => ticket.id === id)!);
      return {
        id: `trash-preview-${request.ticketIds[0]}`,
        destinationDirectory: request.destinationDirectory,
        rows: batchTickets.map((ticket) => ({
          ticketId: ticket.id,
          error: null,
          skipped: false,
          preview: {
            id: `op-${ticket.id}`, fileId: ticket.id, operation_type: "move_to_trash",
            source_path: ticket.path, target_path: `${ticket.path.slice(0, ticket.path.lastIndexOf("/"))}/.zen-canvas-trash/${ticket.name}`,
            old_name: ticket.name, new_name: ticket.name, status: "pending", risk_level: "Normal",
            confidence: 1, requires_confirmation: true, suggested_action: "Move", reason: "safe trash",
            operationFingerprint: `fingerprint-${ticket.id}`
          } as OperationPreview
        }))
      };
    });
    vi.mocked(api.executeTransfer).mockImplementation(async () => ({
      batch_id: `batch-${batchTickets.length}`,
      logs: batchTickets.map((ticket) => ({
        id: `log-${ticket.id}`, batch_id: `batch-${ticket.id}`, operation_type: "move_to_trash",
        source_path: ticket.path, target_path: `${ticket.path}.trash`, old_name: ticket.name, new_name: ticket.name,
        status: "success", error_message: null, created_at: "2026-11-13", can_undo: true,
        path_before: ticket.path, path_after: `${ticket.path}.trash`, name_before: ticket.name,
        name_after: ticket.name, can_restore: true, restored_at: null, restore_status: "not_restored", restore_error: null
      } as OperationLog))
    }));

    await render();
    await click(t("fmMore"));
    await click(t("fmSafeDelete"));
    for (let attempt = 0; attempt < 20 && vi.mocked(api.executeTransfer).mock.calls.length < 2; attempt += 1) await settle();

    expect(api.captureTransferSelectionPage).toHaveBeenNthCalledWith(1, {
      sessionId: "browse-session", snapshotId: "directory-snapshot", offset: 0, excludedEntryIds: [], advanceSnapshot: true
    });
    expect(api.captureTransferSelectionPage).toHaveBeenNthCalledWith(2, {
      sessionId: "browse-session", snapshotId: "directory-snapshot", offset: 1000, excludedEntryIds: [], advanceSnapshot: true
    });
    expect(api.executeTransfer).toHaveBeenCalledTimes(2);
    expect(refresh).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("processed 1001/1001");
  });

  it("moves a persistent basket larger than 1,000 entries in bounded batches", async () => {
    const entries: PersistentBasketEntry[] = Array.from({ length: 1001 }, (_, index) => ({
      id: `basket-${index}`, name: `file-${index}.txt`, path: `C:/Source/file-${index}.txt`,
      isDirectory: false, status: "available"
    }));
    const byId = new Map<string, TransferTicket>();
    entries.forEach((entry) => byId.set(entry.id, { id: `ticket-${entry.id}`, name: entry.name, path: entry.path, isDirectory: false }));
    useFileManagementStore.setState({ basketOpen: true, basketTotal: entries.length, basket: entries.slice(0, 40) });
    vi.mocked(api.basketPage).mockImplementation(async ({ offset, limit }) => ({ entries: entries.slice(offset, offset + limit), total: entries.length }));
    vi.mocked(api.basketCapture).mockImplementation(async ({ entryIds }) => entryIds.map((id) => byId.get(id)!));
    let batchTickets: TransferTicket[] = [];
    vi.mocked(api.previewTransfer).mockImplementation(async (request) => {
      batchTickets = request.ticketIds.map((id) => [...byId.values()].find((ticket) => ticket.id === id)!);
      return {
        id: `basket-preview-${request.ticketIds[0]}`,
        destinationDirectory: request.destinationDirectory,
        rows: batchTickets.map((ticket) => ({
          ticketId: ticket.id, error: null, skipped: false,
          preview: {
            id: `op-${ticket.id}`, fileId: ticket.id, operation_type: request.action as unknown as OperationPreview["operation_type"],
            source_path: ticket.path, target_path: `${request.destinationDirectory}/${ticket.name}`,
            old_name: ticket.name, new_name: ticket.name, status: "pending", risk_level: "Normal",
            confidence: 1, requires_confirmation: true, suggested_action: request.action === "move" ? "Move" : "Copy", reason: "basket transfer",
            operationFingerprint: `fingerprint-${ticket.id}`
          } as OperationPreview
        }))
      };
    });
    vi.mocked(api.executeTransfer).mockImplementation(async () => ({
      batch_id: `basket-batch-${batchTickets.length}`,
      logs: batchTickets.map((ticket) => ({
        id: `log-${ticket.id}`, batch_id: `basket-batch-${ticket.id}`, operation_type: "move",
        source_path: ticket.path, target_path: `C:/Target/${ticket.name}`, old_name: ticket.name, new_name: ticket.name,
        status: "success", error_message: null, created_at: "2026-11-13", can_undo: true,
        path_before: ticket.path, path_after: `C:/Target/${ticket.name}`, name_before: ticket.name,
        name_after: ticket.name, can_restore: true, restored_at: null, restore_status: "not_restored", restore_error: null,
        source_platform_file_id: `source-${ticket.id}`, source_platform_volume_id: "volume",
        target_platform_file_id: `target-${ticket.id}`, target_platform_volume_id: "volume"
      } as OperationLog))
    }));

    await render();
    await click(t("fmMoveBasket"));
    for (let attempt = 0; attempt < 20 && vi.mocked(api.executeTransfer).mock.calls.length < 2; attempt += 1) await settle();

    expect(api.basketCapture).toHaveBeenNthCalledWith(1, { entryIds: entries.slice(0, 1000).map((entry) => entry.id) });
    expect(api.basketCapture).toHaveBeenNthCalledWith(2, { entryIds: [entries[1000]!.id] });
    expect(api.executeTransfer).toHaveBeenCalledTimes(2);
    expect(api.basketReconcileMoves).toHaveBeenCalledTimes(2);
    expect(useFileManagementStore.getState().basketTotal).toBe(1001);
    expect(document.body.textContent).toContain("processed 1001/1001");
  });

  it("compresses the selected identity tickets through the ZIP operation journal", async () => {
    await render();
    await click(t("fmMore"));
    await click(t("fmCompressSelection"));
    expect(api.captureTransfer).toHaveBeenCalledWith({ selection: { kind: "explicit", fileIds: ["managed-one", "managed-two"] } });
    expect(api.createZip).toHaveBeenCalledWith({ ticketIds: ["one", "two"], outputPath: "C:/Target/one-archive.zip", compressionLevel: 6 });
    expect(useFileManagementStore.getState().results[0]?.operation_type).toBe("zip_create");
    expect(useFileManagementStore.getState().results[0]?.can_restore).toBe(false);
    expect(useOperationQueueStore.getState().activeOperationKind).toBe(null);
  });
  it.each(["Control", "Meta"])("uses backend IDs for %s shortcuts and automatically executes a fresh preview", async (modifier) => {
    await render();
    const input=document.createElement("input");document.body.appendChild(input);input.focus();
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown",{key:"c",ctrlKey:modifier === "Control",metaKey:modifier === "Meta",bubbles:true,cancelable:true})); });
    expect(api.captureTransfer).not.toHaveBeenCalled();
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown",{key:"c",ctrlKey:modifier === "Control",metaKey:modifier === "Meta",bubbles:true,cancelable:true})); });
    expect(api.captureTransfer).toHaveBeenCalledWith({selection:{kind:"explicit",fileIds:["managed-one","managed-two"]}});
    await settle();
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown",{key:"v",ctrlKey:modifier === "Control",metaKey:modifier === "Meta",bubbles:true,cancelable:true})); });
    await settle();
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
    expect(api.previewTransfer).toHaveBeenCalledWith({ticketIds:["one","two"],action:"copy",destinationDirectory:"C:/Target",skip:[],keepBoth:[]});
    expect(api.executeTransfer).toHaveBeenCalledWith({previewId:"authority-preview",confirmed:true});
    expect(refresh).toHaveBeenCalledOnce();expect(clear).toHaveBeenCalledOnce();expect(mocks.close).toHaveBeenCalled();
  });
  it("duplicates a same-folder selection through the Copy executor and records the duplicate action", async () => {
    const duplicateTickets: TransferTicket[] = [
      { id: "one", name: "one.txt", path: "C:/Source/one.txt", isDirectory: false },
      { id: "two", name: "two.txt", path: "C:/Source/two.txt", isDirectory: false },
    ];
    vi.mocked(api.captureTransfer).mockResolvedValueOnce(duplicateTickets);
    const duplicatePreview = plan();
    duplicatePreview.id = "duplicate-preview";
    duplicatePreview.destinationDirectory = "C:/Source";
    duplicatePreview.rows.forEach((row, index) => {
      row.preview!.operation_type = "duplicate";
      row.preview!.suggested_action = "Duplicate";
      row.preview!.source_path = duplicateTickets[index]!.path;
      row.preview!.target_path = `C:/Source/${duplicateTickets[index]!.name.replace(".txt", " (2).txt")}`;
    });
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(duplicatePreview);
    vi.mocked(api.executeTransfer).mockResolvedValueOnce({
      batch_id: "duplicate-batch",
      logs: result(["success", "success"]).map((log, index) => ({
        ...log,
        operation_type: "duplicate",
        path_after: duplicatePreview.rows[index]!.preview!.target_path,
      })),
    });

    await render();
    await click(t("fmMore"));
    await click(t("fmDuplicate"));

    expect(api.previewTransfer).toHaveBeenCalledWith({
      ticketIds: ["one", "two"],
      action: "duplicate",
      destinationDirectory: "C:/Source",
      skip: [],
      keepBoth: ["one", "two"],
    });
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "duplicate-preview", confirmed: true });
    expect(useFileManagementStore.getState().results.every((log) => log.operation_type === "duplicate")).toBe(true);
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
  });
  it("keeps mixed-folder selections from being duplicated into one source folder", async () => {
    await render();
    await click(t("fmMore"));
    await click(t("fmDuplicate"));

    expect(api.previewTransfer).not.toHaveBeenCalled();
    expect(document.querySelector('[role="alert"]')?.textContent).toBe(t("fmDuplicateSingleFolder"));
  });
  it.each([
    ["win32", "windows", "copy", "D:/"],
    ["win32", "windows", "move", "D:\\"],
    ["darwin", "macos", "copy", "/"],
    ["darwin", "macos", "move", "/Volumes/USB/"],
  ])("finishes a multi-select %s %s %s workflow and opens %s before History", async (platform, wirePlatform, action, destination) => {
    mocks.platform = platform;
    const refs = tickets.map((ticket) => ({ kind: "ephemeral" as const, browseSessionId: "source-session", entryId: ticket.id }));
    const source = { signature: "browse-multi-selection", count: 2, refs, browse: { sessionId: "source-session", pathRef: { id: "source" } }, routingHint: platform === "darwin" ? "/Source" : "C:/Source", refresh, clear };
    useFileManagementStore.setState({ surface: () => source });
    const preview = plan();
    preview.destinationDirectory = destination;
    preview.rows.forEach((row) => { row.preview!.target_path = `${destination}${row.ticketId}.txt`; row.preview!.operation_type = action as "copy" | "move"; });
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(preview);
    vi.mocked(api.executeTransfer).mockResolvedValueOnce({ batch_id: "batch", logs: result(["success", "success"]).map((log, index) => ({ ...log, operation_type: action, path_after: `${destination}${tickets[index]!.name}` })) });
    await render();
    await click(t(action === "move" ? "fmCut" : "fmCopy"));
    expect(api.captureTransfer).toHaveBeenCalledWith({ refs });
    useFileManagementStore.setState({ surface: () => ({ signature: "empty-destination", count: 0, browse: { sessionId: "target", pathRef: { id: "target" } }, routingHint: destination, refresh, clear }) });
    await render();
    await click(t("fmPaste"));
    expect(api.previewTransfer).toHaveBeenCalledWith(expect.objectContaining({ action, destinationDirectory: destination, ticketIds: ["one", "two"] }));
    expect(api.executeTransfer).toHaveBeenCalledOnce();
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
    expect(mocks.refreshHistory).toHaveBeenCalled();
    expect(useFileManagementStore.getState().clipboard?.tickets).toHaveLength(action === "move" ? 0 : 2);
    await click(t("fmResults"));
    await click(t("fmOpenDestination"));
    const expectedDirectory = destination === "D:/" || destination === "/" ? destination : destination.replace(/\/$/, "");
    expect(mocks.navigate).toHaveBeenCalledWith({ platform: wirePlatform, routingHint: expectedDirectory, displayHint: expectedDirectory });
    await click(t("fmResults"));
    await click(t("fmHistory"));
    expect(mocks.setView).toHaveBeenCalledWith("restore");
  });
  it("opens the successful result directory after a partial failure", async () => {
    const logs = result(["failed", "success"]);
    logs[0]!.path_after = "C:/Missing/one.txt";
    logs[1]!.path_after = "C:/Completed/two.txt";
    useFileManagementStore.setState({ results: logs });
    await render(); await click(t("fmResults")); await click(t("fmOpenDestination"));
    expect(mocks.navigate).toHaveBeenCalledWith(expect.objectContaining({ routingHint: "C:/Completed" }));
  });
  it("keeps History reachable and disables destination navigation when every item failed", async () => {
    useFileManagementStore.setState({ results: result(["failed", "failed"]) });
    await render(); await click(t("fmResults"));
    expect(button(t("fmOpenDestination")).disabled).toBe(true);
    await click(t("fmHistory"));
    expect(mocks.setView).toHaveBeenCalledWith("restore");
    expect(mocks.navigate).not.toHaveBeenCalled();
  });
  it("waits for preview handle release before invoking a mutation", async () => {
    useFileManagementStore.setState({clipboard:{action:"copy",tickets}});
    let release!: () => void;
    mocks.close.mockImplementationOnce(() => new Promise<void>((resolve) => { release=resolve; }));
    await render(); await click(`${t("fmPaste")} (2)`);
    await settle();
    expect(api.executeTransfer).not.toHaveBeenCalled();
    await act(async () => { release(); });
    await settle();
    expect(api.executeTransfer).toHaveBeenCalledOnce();
  });
  it("allows Ctrl+V and toolbar Paste into an empty destination", async () => {
    useFileManagementStore.setState({
      clipboard: { action: "copy", tickets },
      surface: () => ({ signature: "empty-target", count: 0, browse: { sessionId: "empty", pathRef: { id: "empty-root" } }, routingHint: "C:/Empty", refresh, clear }),
    });
    await render();
    expect(button(t("fmPaste")).disabled).toBe(false);
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown", { key: "v", ctrlKey: true, bubbles: true, cancelable: true })); });
    await settle();
    expect(api.previewTransfer).toHaveBeenCalledWith(expect.objectContaining({ destinationDirectory: "C:/Empty" }));
    expect(api.executeTransfer).toHaveBeenCalledOnce();
  });
  it("routes a Windows folder Cut and Paste through the shared Move Preview", async () => {
    vi.mocked(api.captureTransfer).mockResolvedValueOnce([folderTicket]);
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(folderPlan("move"));
    useFileManagementStore.setState({ surface: () => ({ signature: "folder-selection", count: 1, hasDirectorySelection: true, selection: { kind: "explicit", fileIds: ["managed-folder"] }, browse: { sessionId: "source", pathRef: { id: "source" } }, routingHint: "C:/Target", refresh, clear }) });
    await render();
    await click(t("fmCut"));
    expect(useFileManagementStore.getState().clipboard?.action).toBe("move");
    await click(`${t("fmPaste")} (1)`);

    expect(api.previewTransfer).toHaveBeenCalledWith({ ticketIds: ["folder"], action: "move", destinationDirectory: "C:/Target", skip: [], keepBoth: [] });
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "folder-move-preview", confirmed: true });
  });
  it("executes a Windows cross-volume folder Move through the shared Move Preview", async () => {
    useFileManagementStore.setState({ clipboard: { action: "move", tickets: [folderTicket] }, surface: () => ({ signature: "target", count: 0, browse: { sessionId: "target", pathRef: { id: "target" } }, routingHint: "E:/Target", refresh, clear }) });
    vi.mocked(api.previewTransfer).mockReset().mockResolvedValueOnce(folderPlan("move", "E:/Target"));
    await render();
    await click(`${t("fmPaste")} (1)`);

    expect(api.previewTransfer).toHaveBeenCalledWith({ ticketIds: ["folder"], action: "move", destinationDirectory: "E:/Target", skip: [], keepBoth: [] });
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "folder-move-preview", confirmed: true });
  });
  it("shows backend progress and unknown bytes while a transfer is running", async () => {
    useFileManagementStore.setState({ clipboard: { action: "copy", tickets } });
    let resolve!: (value: any) => void;
    vi.mocked(api.executeTransfer).mockImplementationOnce(() => new Promise((r) => { resolve = r; }));
    await render();
    await click(`${t("fmPaste")} (2)`);
    await settle();
    expect(document.querySelector(".fm-task-progress")?.textContent).toContain(t("fmTaskBytesUnknown"));
    expect(document.querySelector(".fm-task-progress progress")?.hasAttribute("value")).toBe(false);
    useOperationQueueStore.setState({ operationProgress: { kind: "execute", batchId: "batch", processed: 1, total: 2, currentPath: "C:/Source/one.txt" } });
    await render();
    expect(document.querySelector(".fm-task-progress")?.textContent).toContain(`1/2 · ${t("fmTaskItems")}`);
    expect(document.querySelector(".fm-task-progress")?.textContent).toContain("C:/Source/one.txt");
    expect(document.querySelector(".fm-task-progress")?.textContent).toContain(t("fmTaskBytesUnknown"));
    useOperationQueueStore.setState({ operationProgress: { kind: "execute", batchId: "batch", processed: 0, total: 1, currentPath: "C:/Source/Folder/nested.bin", phase: "copying", processedItems: 1, totalItems: 4, processedBytes: 4, totalBytes: 10 } });
    await render();
    expect(document.querySelector(".fm-task-progress strong")?.textContent).toBe(t("fmTaskPhaseCopying"));
    expect(document.querySelector(".fm-task-progress progress")?.getAttribute("max")).toBe("10");
    expect(document.querySelector(".fm-task-progress progress")?.getAttribute("value")).toBe("4");
    expect(document.querySelector(".fm-task-progress")?.textContent).toContain("4 B / 10 B");
    await click(t("fmTasks"));
    expect(document.querySelector(".fm-task-entry progress")?.getAttribute("max")).toBe("10");
    expect(document.querySelector(".fm-task-entry progress")?.getAttribute("value")).toBe("4");
    await click(t("fmCancelRunning"));
    await act(async () => { resolve({ batch_id: "batch", logs: result(["failed", "failed"]) }); });
    await settle();
  });
  it("shows recovered Journal work for review and only retries a rolled-back item with fresh capture", async () => {
    const interrupted: OperationLog = { ...result(["failed", "success"])[0]!, status: "manual_review", operation_phase: "source_cleanup_pending", error_message: "target_committed_source_cleanup_pending" };
    useOperationQueueStore.setState({ operationLogs: [interrupted] });
    await render();
    await click(t("fmTasks"));
    expect(document.body.textContent).toContain(t("fmTaskNeedsReview"));
    expect(document.body.textContent).toContain(t("fmTaskPhaseCleanupPending"));
    expect([...document.querySelectorAll("button")].some((item) => item.textContent?.trim() === t("fmRetrySafe"))).toBe(false);

    const retryable: OperationLog = { ...interrupted, status: "failed", operation_phase: "rolled_back", operation_type: "copy", error_message: "source_changed" };
    useOperationQueueStore.setState({ operationLogs: [retryable] });
    await render();
    expect(button(t("fmRetrySafe"))).toBeTruthy();
    vi.mocked(api.captureTransfer).mockResolvedValueOnce([tickets[0]!]);
    await click(t("fmRetrySafe"));
    expect(api.captureTransfer).toHaveBeenCalledWith({ externalPaths: ["C:/Source/one.txt"] });
    expect(api.previewTransfer).toHaveBeenCalledWith({ ticketIds: ["one"], action: "copy", destinationDirectory: "C:/Target", skip: [], keepBoth: [] });
  });
  it("keeps recovery-required results distinct from failed operations", async () => {
    const logs = result(["success", "manual_review"]);
    logs[1]!.operation_phase = "source_cleanup_pending";
    logs[1]!.error_message = "target_committed_source_cleanup_pending";
    useFileManagementStore.setState({ results: logs });
    await render(); await click(t("fmResults"));
    const rows = document.querySelectorAll(".fm-results li");
    expect(rows[1]!.querySelector(".fm-item-status")?.textContent).toBe(t("fmTaskNeedsReview"));
    expect(button(t("fmHistory"))).toBeTruthy();
    expect([...document.querySelectorAll("button")].some((item) => item.textContent?.trim() === t("fmRetrySafe"))).toBe(false);
  });
  it.each([
    ["failed", "fmTaskPartial"],
    ["manual_review", "fmTaskNeedsReview"],
  ] as const)("shows an honest batch outcome when a Move partly returns %s", async (remaining, label) => {
    const logs = result(["success", remaining]);
    vi.mocked(api.basketPage).mockResolvedValue({ entries: basketEntries, total: 2 });
    useFileManagementStore.setState({ clipboard: { action: "move", tickets }, basket: basketEntries, basketTotal: 2 });
    vi.mocked(api.executeTransfer).mockResolvedValueOnce({ batch_id: "batch", logs });
    await render(); await click(t("fmPaste"));
    expect(document.querySelector(".fm-notice")?.textContent).toContain(t(label));
    expect(useFileManagementStore.getState().clipboard?.tickets.map((ticket) => ticket.id)).toEqual(["two"]);
    expect(useFileManagementStore.getState().basket.map((entry) => entry.id)).toEqual(["basket-one", "basket-two"]);
    expect(useFileManagementStore.getState().transferActive).toBe(false);
    expect(useOperationQueueStore.getState().activeOperationKind).toBeNull();
  });
  it("refreshes History and the file surface when an accepted execution returns an uncertain error", async () => {
    useFileManagementStore.setState({ clipboard: { action: "copy", tickets } });
    vi.mocked(api.executeTransfer).mockRejectedValueOnce(new Error("journal outcome unavailable"));
    await render(); await click(`${t("fmPaste")} (2)`);
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
    expect(document.body.textContent).toContain(t("fmExecutionUncertain"));
    expect(mocks.refreshHistory).toHaveBeenCalledOnce();
    expect(refresh).toHaveBeenCalledOnce();
  });
  it("cancels preview preparation without canceling unrelated file operations", async () => {
    useFileManagementStore.setState({clipboard:{action:"copy",tickets}});
    let resolve!: (value: TransferPreview) => void;
    vi.mocked(api.previewTransfer).mockImplementationOnce(() => new Promise((r) => { resolve=r; }));
    await render(); await click(`${t("fmPaste")} (2)`); await click(t("fmCancelRunning"));
    expect(tauriApi.cancelOperations).not.toHaveBeenCalled();
    await act(async () => { resolve(plan()); });
    await settle();
    expect(document.querySelector('[aria-modal="true"]')).toBeNull(); expect(api.executeTransfer).not.toHaveBeenCalled();
  });

  it.each(["failed", "skipped"])("keeps %s cut/basket references after partial success and requests normal cancellation", async (remainingStatus) => {
    vi.mocked(api.basketPage).mockResolvedValue({ entries: basketEntries, total: 2 });
    useFileManagementStore.setState({basket:basketEntries,basketTotal:2,clipboard:{action:"move",tickets},basketOpen:true});
    let resolve!: (value: any) => void;vi.mocked(api.executeTransfer).mockImplementation(() => new Promise((r)=>{resolve=r;}));
    await render();await click(`${t("fmPaste")} (2)`); await settle();
    await click(t("fmCancelRunning"));expect(mocks.cancelOperations).toHaveBeenCalledOnce();
    await act(async()=>{resolve({batch_id:"batch",logs:result(["success",remainingStatus])});});
    await settle();
    expect(useFileManagementStore.getState().basket.map((entry)=>entry.id)).toEqual(["basket-one","basket-two"]);
    expect(useFileManagementStore.getState().clipboard?.tickets.map((ticket)=>ticket.id)).toEqual(["two"]);
    expect(useFileManagementStore.getState().statuses.two).toContain(remainingStatus === "failed" ? "source_locked" : t("fmSkipped"));
    expect(useFileManagementStore.getState().transferActive).toBe(false);
    expect(useOperationQueueStore.getState().activeOperationKind).toBeNull();
    const nextPlan = plan(); nextPlan.rows = [nextPlan.rows[1]!];
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(nextPlan);
    vi.mocked(api.executeTransfer).mockResolvedValueOnce({ batch_id: "retry-batch", logs: [result(["failed", "success"])[1]!] });
    await click(t("fmPaste"));
    expect(api.executeTransfer).toHaveBeenCalledTimes(2);
    expect(mocks.cancelOperations).toHaveBeenCalledTimes(1);
    expect(useFileManagementStore.getState().clipboard?.tickets).toHaveLength(0);
    expect(useFileManagementStore.getState().basket).toHaveLength(2);
    expect(document.querySelector(".fm-notice")?.textContent).toContain(t("fmFinished"));
  });
  it("reports stale sources nonmodally and keeps the persistent basket tickets", async()=>{
    vi.mocked(api.basketPage).mockResolvedValue({ entries: basketEntries, total: 2 });
    useFileManagementStore.setState({basket:basketEntries,basketTotal:2,basketOpen:true});
    vi.mocked(api.basketCapture).mockResolvedValueOnce(tickets);
    const conflict=plan();conflict.rows[0]={ticketId:"one",preview:null,error:"transfer_source_changed",skipped:false};
    vi.mocked(api.previewTransfer).mockResolvedValue(conflict);
    await render();await click(t("fmCopyBasket"));
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();expect(document.body.textContent).toContain(t("fmStale"));
    expect(useFileManagementStore.getState().statuses.one).toContain(t("fmStale"));
    expect(api.executeTransfer).not.toHaveBeenCalled();expect(useFileManagementStore.getState().basket).toHaveLength(2);
  });
  it("reports permission errors without opening a decision modal", async()=>{
    useFileManagementStore.setState({ clipboard: { action: "copy", tickets } });
    vi.mocked(api.previewTransfer).mockRejectedValueOnce(new Error("permission_denied: target is not writable"));
    await render(); await click(`${t("fmPaste")} (2)`);
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
    expect(document.body.textContent).toContain("permission_denied");
    expect(api.executeTransfer).not.toHaveBeenCalled();
  });
  it("keeps auxiliary controls on demand and restores focus without clearing selection", async () => {
    vi.mocked(api.basketPage).mockResolvedValue({ entries: basketEntries, total: 2 });
    await render();
    expect(document.querySelector("#fm-auxiliary-panel")).toBeNull();
    button(t("fmMore")).focus();
    await click(t("fmMore"));
    expect(document.querySelector("#fm-auxiliary-panel")).toBeTruthy();
    await click(t("fmAddBasket"));
    expect(useFileManagementStore.getState().basket).toHaveLength(2);
    expect(useFileManagementStore.getState().basketOpen).toBe(false);
    expect(document.querySelector("#fm-auxiliary-panel")).toBeNull();
    await click(`${t("fmMore")}2`);
    await click(`${t("fmBasket")} (2)`);
    expect(document.querySelector("#fm-auxiliary-panel")?.getAttribute("aria-label")).toBe(t("fmBasket"));
    await act(async () => { document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    expect(document.querySelector("#fm-auxiliary-panel")).toBeNull();
    expect(document.activeElement?.textContent).toContain(t("fmMore"));
    expect(clear).not.toHaveBeenCalled();
  });

  it("persists basket membership and uses fresh identity tickets without clearing after transfer", async () => {
    vi.mocked(api.basketPage).mockResolvedValue({ entries: basketEntries, total: 2 });
    await render();
    await click(t("fmMore"));
    await click(t("fmAddBasket"));
    expect(api.basketAdd).toHaveBeenCalledWith({ ticketIds: ["one", "two"] });
    expect(api.captureTransfer).toHaveBeenCalledWith({ selection: { kind: "explicit", fileIds: ["managed-one", "managed-two"] } });

    await click(`${t("fmMore")}2`);
    await click(`${t("fmBasket")} (2)`);
    await click(t("fmMoveBasket"));
    expect(api.basketCapture).toHaveBeenCalledWith({ entryIds: ["basket-one", "basket-two"] });
    expect(api.basketReconcileMoves).toHaveBeenCalledWith({ moves: [
      { entryId: "basket-one", operationLogId: "log-one" },
      { entryId: "basket-two", operationLogId: "log-two" },
    ] });
    expect(api.basketClear).not.toHaveBeenCalled();
    expect(useFileManagementStore.getState().basket.map((entry) => entry.id)).toEqual(["basket-one", "basket-two"]);
  });

  it("pauses only for a real name conflict and re-previews the whole batch after Skip", async () => {
    useFileManagementStore.setState({ clipboard: {action:"copy", tickets} });
    const first = plan(); first.rows[0] = { ticketId:"one", preview:null, error:"transfer_target_exists", skipped:false };
    const second = plan(); second.rows[0] = { ticketId:"one", preview:null, error:null, skipped:true };
    vi.mocked(api.previewTransfer).mockReset().mockResolvedValueOnce(first).mockResolvedValueOnce(second);
    await render(); await click(`${t("fmPaste")} (2)`);
    expect(document.querySelector('[aria-modal="true"]')).toBeTruthy();expect(api.executeTransfer).not.toHaveBeenCalled();
    const resolution = document.querySelector<HTMLSelectElement>('.fm-conflicts select')!;
    await act(async () => { resolution.value = "skip"; resolution.dispatchEvent(new Event("change", {bubbles:true})); });
    await click(t("fmApplyChoices"));
    expect(api.previewTransfer).toHaveBeenNthCalledWith(2,expect.objectContaining({ticketIds:["one","two"],skip:["one"],keepBoth:[]}));
    expect(api.executeTransfer).toHaveBeenCalledWith({previewId:"authority-preview",confirmed:true});
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
  });

  it("re-previews the whole batch and executes the newly generated Keep Both plan", async () => {
    useFileManagementStore.setState({ clipboard: { action:"copy", tickets } });
    const first = plan(); first.rows[0] = { ticketId:"one", preview:null, error:"transfer_target_exists", skipped:false };
    const fresh = plan(); fresh.id = "fresh-keep-both";
    fresh.rows[0]!.preview = { ...fresh.rows[0]!.preview!, new_name:"one (2).txt", target_path:"C:/Target/one (2).txt" };
    vi.mocked(api.previewTransfer).mockReset().mockResolvedValueOnce(first).mockResolvedValueOnce(fresh);
    await render(); await click(`${t("fmPaste")} (2)`);
    const resolution = document.querySelector<HTMLSelectElement>(".fm-conflicts select")!;
    await act(async () => { resolution.value = "both"; resolution.dispatchEvent(new Event("change", {bubbles:true})); });
    await click(t("fmApplyChoices"));
    expect(api.previewTransfer).toHaveBeenNthCalledWith(2,expect.objectContaining({ticketIds:["one","two"],skip:[],keepBoth:["one"]}));
    expect(api.executeTransfer).toHaveBeenCalledWith({previewId:"fresh-keep-both",confirmed:true});
  });

  it("creates a folder through automatic Preview and execution without a second confirmation", async () => {
    useFileManagementStore.setState({ surface: () => ({ signature: "browse", count: 0, browse: { sessionId: "browse", pathRef: { id: "root" } }, routingHint: "C:/Browse", refresh, clear }) });
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(actionPlan(tickets[0]!, "create_directory", "C:/Browse", "New Folder"));
    await render();
    await click(t("fmNewFolder"));
    const dialog = document.querySelector<HTMLElement>("[aria-modal=true]")!;
    await fill(dialog.querySelector("input")!, "New Folder");
    await act(async () => { dialog.querySelector<HTMLButtonElement>('button[type="submit"]')!.click(); });
    await settle();

    expect(api.captureTransfer).not.toHaveBeenCalled();
    expect(api.previewTransfer).toHaveBeenCalledWith(expect.objectContaining({
      ticketIds: [], action: "create_directory", destinationDirectory: "C:/Browse", newName: "New Folder", skip: [], keepBoth: []
    }));
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "authority-create_directory", confirmed: true });
    expect(document.querySelector("[aria-modal=true]")).toBeNull();
    expect(clear).not.toHaveBeenCalled();
  });

  it("renames the selected item through the orchestrator and re-previews only after a name conflict", async () => {
    useFileManagementStore.setState({ surface: () => ({ signature: "one-selected", count: 1, selection: { kind: "explicit", fileIds: ["managed-one"] }, browse: { sessionId: "session", pathRef: { id: "root" } }, routingHint: "C:/Source", refresh, clear }) });
    vi.mocked(api.captureTransfer).mockResolvedValueOnce([tickets[0]!]);
    const conflict = actionPlan(tickets[0]!, "rename", "C:/Source", "renamed.txt");
    conflict.rows[0] = { ticketId: "one", preview: null, error: "transfer_target_exists", skipped: false };
    vi.mocked(api.previewTransfer).mockReset().mockResolvedValueOnce(conflict).mockResolvedValueOnce(actionPlan(tickets[0]!, "rename", "C:/Source", "renamed-again.txt"));

    await render();
    await click(t("fmMore"));
    await click(t("fmRename"));
    const entryDialog = document.querySelector<HTMLElement>("[aria-modal=true]")!;
    await fill(entryDialog.querySelector("input")!, "renamed.txt");
    await act(async () => { entryDialog.querySelector<HTMLButtonElement>('button[type="submit"]')!.click(); });
    await settle();

    const conflictDialog = document.querySelector<HTMLElement>(".fm-conflict-dialog[aria-modal=true]")!;
    expect(conflictDialog.querySelector(".fm-conflicts select")).toBeNull();
    await fill(conflictDialog.querySelector("input")!, "renamed-again.txt");
    await act(async () => { conflictDialog.querySelector<HTMLButtonElement>(".fm-confirm-bar button:last-child")!.click(); });
    await settle();

    expect(api.captureTransfer).toHaveBeenCalledWith({ selection: { kind: "explicit", fileIds: ["managed-one"] } });
    expect(api.previewTransfer).toHaveBeenNthCalledWith(1, expect.objectContaining({
      ticketIds: ["one"], action: "rename", destinationDirectory: "C:/Source", newName: "renamed.txt"
    }));
    expect(api.previewTransfer).toHaveBeenNthCalledWith(2, expect.objectContaining({
      ticketIds: ["one"], action: "rename", destinationDirectory: "C:/Source", newName: "renamed-again.txt"
    }));
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "authority-rename", confirmed: true });
    expect(document.querySelector("[aria-modal=true]")).toBeNull();
  });

  it("routes Safe Delete through the same automatic journal transfer path", async () => {
    useFileManagementStore.setState({ surface: () => ({ signature: "one-selected", count: 1, selection: { kind: "explicit", fileIds: ["managed-one"] }, browse: { sessionId: "session", pathRef: { id: "root" } }, routingHint: "C:/Source", refresh, clear }) });
    vi.mocked(api.captureTransfer).mockResolvedValueOnce([tickets[0]!]);
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(actionPlan(tickets[0]!, "trash", "C:/Source", tickets[0]!.name));
    await render();
    await click(t("fmMore"));
    await click(t("fmSafeDelete"));

    expect(api.previewTransfer).toHaveBeenCalledWith(expect.objectContaining({
      ticketIds: ["one"], action: "trash", destinationDirectory: "C:/Source", skip: [], keepBoth: []
    }));
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "authority-trash", confirmed: true });
    expect(document.querySelector("[aria-modal=true]")).toBeNull();
  });

  it("uses the configured system trash destination for Delete", async () => {
    mocks.defaultDeleteDestination = "system_trash";
    await render();
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true })); });
    await settle();
    expect(api.previewTransfer).toHaveBeenCalledWith(expect.objectContaining({ action: "system_trash" }));
    expect(api.executeTransfer).toHaveBeenCalledOnce();
  });

  it("binds cross-pane Copy to the source selection and auto-executes for the captured target", async () => {
    const beforeMutation = vi.fn(async () => undefined);
    const afterMutation = vi.fn(async () => undefined);
    const clearTarget = vi.fn();
    const source = {signature:"left-selected",count:2,selection:{kind:"explicit" as const,fileIds:["managed-one","managed-two"]},browse:{sessionId:"left-session",pathRef:{id:"left-root"}},routingHint:"C:/Source",refresh,clear};
    const target = {signature:"right-empty",count:0,refs:[],browse:{sessionId:"right-session",pathRef:{id:"right-root"}},routingHint:"C:/Target",refresh,clear:clearTarget};
    useFileManagementStore.setState({activePaneId:"left",surface:()=>source,surfaces:{left:{owner:"left",read:()=>source},right:{owner:"right",read:()=>target}}});
    function TwoPanes() {
      const [active,setActive] = useState("left");
      const value = (id: string): PaneContextValue => ({id,visible:true,focused:active===id,rootRef:{current:host},projection:defaultFileLibraryProjection,scope:{kind:"all"},setScope:vi.fn(),routingHint:id==="left"?"C:/Source":"C:/Target",updateLocation:vi.fn(),activate:()=>setActive(id),targetPaneId:id==="left"?"right":"left",activateTarget:()=>{setActive(id==="left"?"right":"left");useFileManagementStore.getState().activatePane(id==="left"?"right":"left");},registerPreview:vi.fn(),beforeMutation,afterMutation});
      return <><PaneContext.Provider value={value("left")}><FileManagementToolbar/></PaneContext.Provider><PaneContext.Provider value={value("right")}><FileManagementToolbar/></PaneContext.Provider></>;
    }
    await act(async () => root.render(<TwoPanes/>));
    await act(async () => useFileManagementStore.getState().requestAction("copy_other"));
    expect(api.captureTransfer).toHaveBeenCalledWith({selection:source.selection});
    await settle();
    expect(api.previewTransfer).toHaveBeenCalledWith({ticketIds:["one","two"],action:"copy",destinationDirectory:"C:/Target",destinationBrowse:{sessionId:"right-session",pathRef:{id:"right-root"}},skip:[],keepBoth:[]});
    expect(api.executeTransfer).toHaveBeenCalledWith({previewId:"authority-preview",confirmed:true});
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
    expect(beforeMutation).toHaveBeenCalledOnce();
    expect(afterMutation).toHaveBeenCalledWith(["C:/Target","C:/Source","C:/Other"]);
    expect(clear).toHaveBeenCalledOnce();expect(clearTarget).not.toHaveBeenCalled();
    expect(useFileManagementStore.getState().transferOpen).toBe(false);
  });

  it.each([
    { action: "copy", change: "projection", shouldExecute: true },
    { action: "move", change: "projection", shouldExecute: true },
    { action: "copy", change: "source-selection", shouldExecute: false },
    { action: "copy", change: "destination-path", shouldExecute: false },
  ] as const)("guards cross-pane $action against $change changes", async ({ action, change, shouldExecute }) => {
    const sourceRefs = [
      { kind: "ephemeral" as const, browseSessionId: "left-session", entryId: "one" },
      { kind: "ephemeral" as const, browseSessionId: "left-session", entryId: "two" },
    ];
    const makeBrowseState = (sessionId: string, pathId: string, routingHint: string, selectedEntryRefs: typeof sourceRefs) => ({
      sessionId, pathId, routingHint, selectedEntryRefs,
      query: { text: null, entryKind: "all" },
      enumerationId: `${sessionId}-enumeration-1`, entriesLength: selectedEntryRefs.length,
      enumerationState: "complete", pendingChange: null, changeState: "watching",
    });
    const sourceState = makeBrowseState("left-session", "left-root", "C:/Source", sourceRefs);
    const targetState = makeBrowseState("right-session", "right-root", "C:/Target", []);
    const makeSurface = (state: typeof sourceState) => ({
      get signature() {
        return JSON.stringify(["browse", state.sessionId, state.pathId, state.query, state.selectedEntryRefs,
          state.enumerationId, state.entriesLength, state.enumerationState, state.pendingChange, state.changeState]);
      },
      get count() { return state.selectedEntryRefs.length; },
      get refs() { return state.selectedEntryRefs; },
      browse: { sessionId: state.sessionId, get pathRef() { return { id: state.pathId }; } },
      get routingHint() { return state.routingHint; },
      refresh, clear,
    });
    const source = makeSurface(sourceState);
    const target = makeSurface(targetState);
    useFileManagementStore.setState({activePaneId:"left",surface:()=>source,surfaces:{left:{owner:"left",read:()=>source},right:{owner:"right",read:()=>target}}});
    const acceptedPreview = plan();
    acceptedPreview.rows.forEach((row) => {
      row.preview!.operation_type = action;
      row.preview!.suggested_action = action === "move" ? "Move" : "Copy";
    });
    vi.mocked(api.previewTransfer).mockImplementationOnce(async (request) => {
      if (change === "projection") {
        // The other pane becoming visible advances Browse enumeration and watcher
        // projection state while the authoritative Preview request is in flight.
        sourceState.enumerationId = "left-session-enumeration-2";
        sourceState.enumerationState = "loading_more";
        targetState.enumerationId = "right-session-enumeration-2";
        targetState.enumerationState = "complete";
        targetState.entriesLength = 3;
      } else if (change === "source-selection") {
        sourceState.selectedEntryRefs = [sourceRefs[0]!];
        sourceState.entriesLength = 1;
      } else {
        targetState.pathId = "right-other-root";
        targetState.routingHint = "C:/OtherTarget";
      }
      return {
        ...acceptedPreview,
        destinationDirectory: request.destinationDirectory,
        rows: acceptedPreview.rows.map((row) => ({
          ...row,
          preview: row.preview ? { ...row.preview, operation_type: action } : null,
        })),
      };
    });
    function TwoPanes() {
      const [active,setActive] = useState("left");
      const value = (id: string): PaneContextValue => ({id,visible:true,focused:active===id,rootRef:{current:host},projection:defaultFileLibraryProjection,scope:{kind:"all"},setScope:vi.fn(),routingHint:id==="left"?"C:/Source":"C:/Target",updateLocation:vi.fn(),activate:()=>setActive(id),targetPaneId:id==="left"?"right":"left",activateTarget:()=>{setActive(id==="left"?"right":"left");useFileManagementStore.getState().activatePane(id==="left"?"right":"left");},registerPreview:vi.fn(),beforeMutation:vi.fn(async()=>undefined),afterMutation:vi.fn(async()=>undefined)});
      return <><PaneContext.Provider value={value("left")}><FileManagementToolbar/></PaneContext.Provider><PaneContext.Provider value={value("right")}><FileManagementToolbar/></PaneContext.Provider></>;
    }

    await act(async () => root.render(<TwoPanes/>));
    await act(async () => useFileManagementStore.getState().requestAction(action === "copy" ? "copy_other" : "move_other"));
    await settle();

    expect(api.captureTransfer).toHaveBeenCalledWith({ refs: sourceRefs });
    expect(api.previewTransfer).toHaveBeenCalledWith(expect.objectContaining({
      ticketIds: ["one", "two"], action, destinationDirectory: "C:/Target",
    }));
    if (shouldExecute) {
      expect(api.executeTransfer).toHaveBeenCalledWith({previewId:"authority-preview",confirmed:true});
      expect(document.body.textContent).not.toContain("transfer_preview_stale");
    } else {
      expect(api.executeTransfer).not.toHaveBeenCalled();
      expect(document.body.textContent).toContain("transfer_preview_stale");
    }
  });

  it("marks unavailable favorite locations and keeps them out of destination selection", async()=>{
    vi.mocked(api.fileLibraryPlaces).mockResolvedValue([{id:"offline",name:"Offline disk",routingHint:"Z:/Gone",favorite:true,current:false,visitedAt:1,unavailable:true}]);
    await render();await click(t("fmMore"));await click(t("fmPlaces"));expect(document.body.textContent).toContain(t("fmUnavailable"));
    const offline=[...document.querySelectorAll<HTMLButtonElement>("button")].find((b)=>b.textContent?.includes("Offline disk"))!;expect(offline.disabled).toBe(true);expect(mocks.navigate).not.toHaveBeenCalled();
  });
});
