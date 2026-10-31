// @vitest-environment happy-dom
import { act, useState } from "react";
import { PaneContext, type PaneContextValue } from "../src/views/fileLibrary/workbench/PaneContext";
import { defaultFileLibraryProjection } from "../src/store/useFileLibraryV2Store";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import { FileManagementToolbar } from "../src/views/fileLibrary/management/FileManagementToolbar";
import { useFileManagementStore } from "../src/views/fileLibrary/management/fileManagementStore";
import { fileManagementApi as api, type TransferTicket, type TransferPreview } from "../src/api/fileManagementApi";
import { tauriApi } from "../src/api/tauriApi";
import type { OperationLog } from "../src/types/domain";
const mocks = vi.hoisted(() => ({ close: vi.fn(), navigate: vi.fn(), open: vi.fn(), refreshHistory: vi.fn(async () => []) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
vi.mock("../src/contexts/AppContexts", async () => {
  const { makeTranslator } = await import("../src/i18n"); const t = makeTranslator("en");
  return { useI18nContext: () => ({ t, language: "en" }), useChromeContext: () => ({ platform: "win32" }), useNavigationContext: () => ({ setView: vi.fn() }) };
});
vi.mock("../src/views/fileLibrary/FileLibraryExperienceProvider", () => ({ useFileLibraryExperience: () => ({ controller: { openBrowse: mocks.navigate } }) }));
vi.mock("../src/views/fileLibrary/preview/PreviewExperienceProvider", () => ({ usePreviewExperience: () => ({ controller: { close: mocks.close, closeForFileOperation: mocks.close } }) }));
vi.mock("../src/store/useOperationQueueStore", () => ({ useOperationQueueStore: { getState: () => ({ refreshOperationLogs: mocks.refreshHistory }) } }));
const t = makeTranslator("en");
const tickets: TransferTicket[] = [{id:"one",name:"one.txt",path:"C:/Source/one.txt",isDirectory:false},{id:"two",name:"two.txt",path:"C:/Other/two.txt",isDirectory:false}];
function plan(): TransferPreview { return { id: "authority-preview", destinationDirectory: "C:/Target", rows: tickets.map((ticket) => ({ticketId: ticket.id, error: null, skipped: false, preview: { id: `op-${ticket.id}`, fileId: ticket.id, operation_type:"copy",source_path:ticket.path,target_path:`C:/Target/${ticket.name}`,old_name:ticket.name,new_name:ticket.name,status:"pending",risk_level:"Normal",confidence:1,requires_confirmation:true,suggested_action:"Copy",reason:"manual",operationFingerprint:"bound-proof"} })) }; }
function singlePlan(ticketId: string, action: "rename" | "move_to_trash" | "create_directory", sourcePath: string, targetPath: string, oldName: string, newName: string): TransferPreview {
  const base = plan().rows[0]!.preview!;
  return { id: "authority-preview", destinationDirectory: "C:/Target", rows: [{ ticketId, error: null, skipped: false, preview: { ...base, id: `op-${ticketId}`, fileId: ticketId, operation_type: action as any, source_path: sourcePath, target_path: targetPath, old_name: oldName, new_name: newName } }] };
}
function result(statuses: string[]): OperationLog[] { return tickets.map((ticket,index) => ({ id:`log-${ticket.id}`,batch_id:"batch",operation_type:"move",source_path:ticket.path,target_path:`C:/Target/${ticket.name}`,old_name:ticket.name,new_name:ticket.name,status:statuses[index],error_message:statuses[index]==="failed"?"source_locked":null,created_at:"2026-10-26",can_undo:false,path_before:ticket.path,path_after:`C:/Target/${ticket.name}`,name_before:ticket.name,name_after:ticket.name,can_restore:false,restored_at:null,restore_status:"not_restored",restore_error:null } as OperationLog)); }
let root: Root; let host: HTMLDivElement;
const refresh = vi.fn(async () => undefined); const clear = vi.fn();
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === text)!;
async function click(text: string) { await act(async () => { button(text).click(); }); }
async function render() { await act(async () => { root.render(<FileManagementToolbar/>); }); }
async function flush() { await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); }); }
async function changeInputValue(input: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  document.body.innerHTML = '<main id="app-shell-content"></main>'; host = document.createElement("div"); document.body.querySelector("main")!.appendChild(host); root = createRoot(host);
  vi.spyOn(api,"captureTransfer").mockResolvedValue(tickets);
  vi.spyOn(api,"fileLibraryPlaces").mockResolvedValue([{id:"place",name:"Target",routingHint:"C:/Target",favorite:true,current:true,visitedAt:1,unavailable:false}]);
  vi.spyOn(api,"previewTransfer").mockResolvedValue(plan());
  vi.spyOn(api,"executeTransfer").mockResolvedValue({batch_id:"batch",logs:result(["success","success"])});
  vi.spyOn(tauriApi,"onOperationProgress").mockResolvedValue(() => undefined);
  vi.spyOn(tauriApi,"cancelOperations").mockResolvedValue();
  useFileManagementStore.setState({owner:"owner",surface:()=>({signature:"selected",count:2,selection:{kind:"explicit",fileIds:["managed-one","managed-two"]},browse:{sessionId:"session",pathRef:{id:"root"}},refresh,clear}),basket:[],clipboard:null,results:[],statuses:{},basketOpen:false,pendingAction:null,surfaces:{},activePaneId:null,pendingTransfer:null,transferOpen:false});
  refresh.mockClear();clear.mockClear();mocks.close.mockClear();
});
afterEach(async () => { await act(async () => root.unmount()); vi.restoreAllMocks(); document.body.innerHTML=""; });
describe("File Library manual management workflow", () => {
  it.each(["Control", "Meta"])("uses backend IDs for %s shortcuts, leaves text editing alone, and requires confirmation after paste preview", async (modifier) => {
    await render();
    const input=document.createElement("input");document.body.appendChild(input);input.focus();
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown",{key:"c",ctrlKey:modifier === "Control",metaKey:modifier === "Meta",bubbles:true,cancelable:true})); });
    expect(api.captureTransfer).not.toHaveBeenCalled();
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown",{key:"c",ctrlKey:modifier === "Control",metaKey:modifier === "Meta",bubbles:true,cancelable:true})); });
    expect(api.captureTransfer).toHaveBeenCalledWith({selection:{kind:"explicit",fileIds:["managed-one","managed-two"]}});
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown",{key:"v",ctrlKey:modifier === "Control",metaKey:modifier === "Meta",bubbles:true,cancelable:true})); });
    await flush();
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    expect(api.previewTransfer).not.toHaveBeenCalled();
    await click(t("fmPrepare"));
    expect(api.previewTransfer).toHaveBeenCalledWith({ticketIds:["one","two"],action:"copy",destinationDirectory:"C:/Target",skip:[],keepBoth:[]});
    expect(api.executeTransfer).not.toHaveBeenCalled();
    await click(t("fmConfirm"));
    expect(api.executeTransfer).toHaveBeenCalledWith({previewId:"authority-preview",confirmed:true});
    expect(refresh).toHaveBeenCalledOnce();expect(clear).toHaveBeenCalledOnce();expect(mocks.close).toHaveBeenCalled();
  });
  it("waits for preview handle release before invoking a confirmed mutation", async () => {
    useFileManagementStore.setState({clipboard:{action:"copy",tickets}});
    let release!: () => void;
    mocks.close.mockImplementationOnce(() => new Promise<void>((resolve) => { release=resolve; }));
    await render(); await click(`${t("fmPaste")} (2)`); await click(t("fmPrepare")); await click(t("fmConfirm")); await flush();
    expect(api.executeTransfer).not.toHaveBeenCalled();
    await act(async () => { release(); });
    expect(api.executeTransfer).toHaveBeenCalledOnce();
  });
  it("picks an explicit destination, previews it, and waits for toolbar Move confirmation", async () => {
    mocks.open.mockResolvedValueOnce("C:/Elsewhere");
    await render();
    await click(t("fmMove"));
    await flush();
    expect(mocks.open).toHaveBeenCalledWith({ directory: true, multiple: false, title: t("fmDestination") });
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    expect(api.previewTransfer).not.toHaveBeenCalled();
    await click(t("fmPrepare"));
    expect(api.previewTransfer).toHaveBeenCalledWith({ ticketIds: ["one", "two"], action: "move", destinationDirectory: "C:/Elsewhere", skip: [], keepBoth: [] });
    expect(api.executeTransfer).not.toHaveBeenCalled();
    await click(t("fmConfirm"));
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "authority-preview", confirmed: true });
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(refresh).toHaveBeenCalledOnce();
  });
  it("cancels preview preparation without canceling unrelated file operations", async () => {
    useFileManagementStore.setState({clipboard:{action:"copy",tickets}});
    let resolve!: (value: TransferPreview) => void;
    vi.mocked(api.previewTransfer).mockImplementationOnce(() => new Promise((r) => { resolve=r; }));
    await render(); await click(`${t("fmPaste")} (2)`); await click(t("fmPrepare")); await click(t("fmCancel"));
    expect(tauriApi.cancelOperations).not.toHaveBeenCalled();
    await act(async () => { resolve(plan()); });
    expect(document.querySelector('[role="dialog"]')).toBeNull(); expect(api.executeTransfer).not.toHaveBeenCalled();
  });

  it.each(["failed", "skipped"])("keeps %s cut/basket references after partial success and requests normal cancellation", async (remainingStatus) => {
    useFileManagementStore.setState({basket:tickets,clipboard:{action:"move",tickets},basketOpen:true});
    let resolve!: (value: any) => void;vi.mocked(api.executeTransfer).mockImplementation(() => new Promise((r)=>{resolve=r;}));
    await render();await click(`${t("fmPaste")} (2)`);await click(t("fmPrepare"));await click(t("fmConfirm"));await flush();
    await click(t("fmCancelRunning"));expect(tauriApi.cancelOperations).toHaveBeenCalledOnce();
    await act(async()=>{resolve({batch_id:"batch",logs:result(["success",remainingStatus])});});
    expect(useFileManagementStore.getState().basket.map((ticket)=>ticket.id)).toEqual(["two"]);
    expect(useFileManagementStore.getState().clipboard?.tickets.map((ticket)=>ticket.id)).toEqual(["two"]);
    expect(useFileManagementStore.getState().statuses.two).toContain(remainingStatus === "failed" ? "source_locked" : t("fmSkipped"));
  });
  it("blocks stale/conflicting references, invalidates consent when options change, and cancels without execute", async()=>{
    useFileManagementStore.setState({basket:tickets,basketOpen:true});
    const conflict=plan();conflict.rows[0]={ticketId:"one",preview:null,error:"transfer_source_changed",skipped:false};
    vi.mocked(api.previewTransfer).mockResolvedValue(conflict);
    await render();await click(t("fmCopyBasket"));await click(t("fmPrepare"));await flush();
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();expect(document.body.textContent).toContain(t("fmStale"));
    expect(useFileManagementStore.getState().statuses.one).toContain(t("fmStale"));
    expect(api.executeTransfer).not.toHaveBeenCalled();expect(useFileManagementStore.getState().basket).toHaveLength(2);
  });
  it("keeps auxiliary controls on demand and restores focus without clearing selection", async () => {
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

  it("makes conflict resolutions exclusive and requires fresh backend consent after every change", async () => {
    useFileManagementStore.setState({ clipboard: {action:"copy", tickets} });
    const conflict = plan();
    conflict.rows[0] = { ticketId: "one", preview: null, error: "transfer_target_exists", skipped: false };
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(conflict);
    await render(); await click(`${t("fmPaste")} (2)`); await click(t("fmPrepare")); await flush();
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    const resolution = document.querySelector<HTMLSelectElement>(".fm-preview select")!;
    await act(async () => { resolution.value = "skip"; resolution.dispatchEvent(new Event("change", {bubbles:true})); });
    expect(button(t("fmConfirm")).disabled).toBe(true);
    await click(t("fmPrepare"));
    expect(api.previewTransfer).toHaveBeenLastCalledWith(expect.objectContaining({skip:["one"],keepBoth:[]}));
    await act(async () => { resolution.value = "both"; resolution.dispatchEvent(new Event("change", {bubbles:true})); });
    expect(button(t("fmConfirm")).disabled).toBe(true);
    await click(t("fmPrepare"));
    expect(api.previewTransfer).toHaveBeenLastCalledWith(expect.objectContaining({skip:[],keepBoth:["one"]}));
    await click(t("fmCancel"));
    expect(api.executeTransfer).not.toHaveBeenCalled();
  });

  it("binds cross-pane Copy to the source selection and target directory, releasing all previews before confirmed execution", async () => {
    const beforeMutation = vi.fn(async () => undefined);
    const afterMutation = vi.fn(async () => undefined);
    const clearTarget = vi.fn();
    const source = {signature:"left-selected",count:2,selection:{kind:"explicit" as const,fileIds:["managed-one","managed-two"]},browse:{sessionId:"left-session",pathRef:{id:"left-root"}},routingHint:"C:/Source",refresh,clear};
    const target = {signature:"right-empty",count:0,refs:[],browse:{sessionId:"right-session",pathRef:{id:"right-root"}},routingHint:"C:/Target",refresh,clear:clearTarget};
    useFileManagementStore.setState({activePaneId:"left",surface:()=>source,surfaces:{left:{owner:"left",read:()=>source},right:{owner:"right",read:()=>target}}});
    function TwoPanes() {
      const [active,setActive] = useState("left");
      const value = (id: string): PaneContextValue => ({id,visible:true,focused:active===id,rootRef:{current:host},projection:defaultFileLibraryProjection,scope:{kind:"all"},setScope:vi.fn(),routingHint:id==="left"?"C:/Source":"C:/Target",updateLocation:vi.fn(),activate:()=>setActive(id),peerId:id==="left"?"right":"left",activatePeer:()=>{setActive(id==="left"?"right":"left");useFileManagementStore.getState().activatePane(id==="left"?"right":"left");},registerPreview:vi.fn(),beforeMutation,afterMutation});
      return <><PaneContext.Provider value={value("left")}><FileManagementToolbar/></PaneContext.Provider><PaneContext.Provider value={value("right")}><FileManagementToolbar/></PaneContext.Provider></>;
    }
    await act(async () => root.render(<TwoPanes/>));
    await act(async () => useFileManagementStore.getState().requestAction("copy_other"));
    expect(api.captureTransfer).toHaveBeenCalledWith({selection:source.selection});
    await flush();
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    expect(api.previewTransfer).not.toHaveBeenCalled();
    await click(t("fmPrepare"));
    expect(api.previewTransfer).toHaveBeenCalledWith({ticketIds:["one","two"],action:"copy",destinationDirectory:"C:/Target",skip:[],keepBoth:[]});
    expect(api.executeTransfer).not.toHaveBeenCalled();
    await click(t("fmConfirm"));
    expect(beforeMutation).toHaveBeenCalledOnce();
    expect(api.executeTransfer).toHaveBeenCalledWith({previewId:"authority-preview",confirmed:true});
    expect(afterMutation).toHaveBeenCalledWith(["C:/Target","C:/Source","C:/Other"]);
    expect(clear).toHaveBeenCalledOnce();expect(clearTarget).not.toHaveBeenCalled();
    expect(useFileManagementStore.getState().transferOpen).toBe(false);
  });

  it("marks unavailable favorite locations and keeps them out of destination selection", async()=>{
    vi.mocked(api.fileLibraryPlaces).mockResolvedValue([{id:"offline",name:"Offline disk",routingHint:"Z:/Gone",favorite:true,current:false,visitedAt:1,unavailable:true}]);
    await render();await click(t("fmMore"));await click(t("fmPlaces"));expect(document.body.textContent).toContain(t("fmUnavailable"));
    const offline=[...document.querySelectorAll<HTMLButtonElement>("button")].find((b)=>b.textContent?.includes("Offline disk"))!;expect(offline.disabled).toBe(true);expect(mocks.navigate).not.toHaveBeenCalled();
  });

  it("creates a folder through Preview and waits for explicit confirmation", async () => {
    const folderPlan = singlePlan("create-directory", "create_directory", "C:/Target", "C:/Target/New folder", "New folder", "New folder");
    vi.mocked(api.previewTransfer).mockResolvedValue(folderPlan);
    await render();
    await click(t("fmNewFolder"));
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    const name = document.querySelector<HTMLInputElement>(".fm-dialog input")!;
    await changeInputValue(name, "QA folder");
    await click(t("fmPrepare"));
    expect(api.previewTransfer).toHaveBeenCalledWith({ ticketIds: [], action: "create_directory", destinationDirectory: "C:/Target", newName: "QA folder", skip: [], keepBoth: [] });
    expect(api.executeTransfer).not.toHaveBeenCalled();
    await click(t("fmConfirm"));
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "authority-preview", confirmed: true });
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("uses F2 for one-item rename and Delete for safe trash, with a prepared preview before execution", async () => {
    const one = tickets[0]!;
    const surface = () => ({ signature: "selected", count: 1, selection: { kind: "explicit" as const, fileIds: ["managed-one"] }, browse: { sessionId: "session", pathRef: { id: "root" } }, refresh, clear });
    useFileManagementStore.setState({ surface });
    vi.mocked(api.captureTransfer).mockResolvedValueOnce([one]);
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(singlePlan(one.id, "rename", one.path, "C:/Source/renamed.txt", one.name, "renamed.txt"));
    await render();
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true, cancelable: true })); });
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    const name = document.querySelector<HTMLInputElement>(".fm-dialog input")!;
    await changeInputValue(name, "renamed.txt");
    await click(t("fmPrepare"));
    expect(api.previewTransfer).toHaveBeenLastCalledWith({ ticketIds: ["one"], action: "rename", destinationDirectory: "C:/Source", newName: "renamed.txt", skip: [], keepBoth: [] });
    expect(api.executeTransfer).not.toHaveBeenCalled();
    await click(t("fmConfirm"));
    expect(api.executeTransfer).toHaveBeenCalledOnce();

    const trashPlan = plan();
    trashPlan.rows = trashPlan.rows.map((row) => ({ ...row, preview: row.preview ? { ...row.preview, operation_type: "move_to_trash" } : null }));
    vi.mocked(api.previewTransfer).mockResolvedValueOnce(trashPlan);
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true })); });
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    await click(t("fmPrepare"));
    expect(api.previewTransfer).toHaveBeenLastCalledWith({ ticketIds: ["one", "two"], action: "trash", destinationDirectory: "C:/Target", skip: [], keepBoth: [] });
    await click(t("fmConfirm"));
    expect(api.executeTransfer).toHaveBeenCalledTimes(2);
  });

  it("shows permission errors in the preview and keeps confirmation disabled", async () => {
    useFileManagementStore.setState({ clipboard: { action: "copy", tickets } });
    const denied = plan();
    denied.rows = [{ ticketId: "one", preview: null, error: "permission denied: access_denied", skipped: false }];
    vi.mocked(api.previewTransfer).mockResolvedValue(denied);
    await render();
    await click(`${t("fmPaste")} (2)`);
    await click(t("fmPrepare"));
    expect(document.body.textContent).toContain("permission denied: access_denied");
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    expect(button(t("fmConfirm")).disabled).toBe(true);
    expect(api.executeTransfer).not.toHaveBeenCalled();
  });

  it("disables Windows directory copy, cut, move and basket actions while keeping rename and safe delete available", async () => {
    const directorySurface = () => ({ signature: "folder", count: 1, hasDirectorySelection: true, browse: { sessionId: "session", pathRef: { id: "root" } }, refresh, clear });
    useFileManagementStore.setState({ surface: directorySurface });
    await render();
    for (const label of [t("fmCopy"), t("fmCut"), t("fmMove")]) expect(button(label).disabled).toBe(true);
    expect(button(t("fmRename")).disabled).toBe(false);
    expect(button(t("fmSafeDelete")).disabled).toBe(false);
    await click(t("fmMore"));
    expect(button(t("fmAddBasket")).disabled).toBe(true);
  });
});
