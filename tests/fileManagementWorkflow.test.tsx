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
import type { OperationLog, OperationPreview } from "../src/types/domain";
const mocks = vi.hoisted(() => ({ close: vi.fn(), navigate: vi.fn(), refreshHistory: vi.fn(async () => []) }));
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
  vi.spyOn(api,"fileLibraryPlaces").mockResolvedValue([{id:"place",name:"Target",routingHint:"C:/Target",favorite:true,current:true,visitedAt:1,unavailable:false}]);
  vi.spyOn(api,"previewTransfer").mockResolvedValue(plan());
  vi.spyOn(api,"executeTransfer").mockResolvedValue({batch_id:"batch",logs:result(["success","success"])});
  vi.spyOn(tauriApi,"onOperationProgress").mockResolvedValue(() => undefined);
  vi.spyOn(tauriApi,"cancelOperations").mockResolvedValue();
  useFileManagementStore.setState({owner:"owner",surface:()=>({signature:"selected",count:2,selection:{kind:"explicit",fileIds:["managed-one","managed-two"]},browse:{sessionId:"session",pathRef:{id:"root"}},refresh,clear}),basket:[],clipboard:null,results:[],statuses:{},basketOpen:false,pendingAction:null,surfaces:{},activePaneId:null,pendingTransfer:null,transferOpen:false,transferActive:false,transferRunning:false,activeTransferId:null,transferTicketIds:[],externalDropAction:"copy"});
  refresh.mockClear();clear.mockClear();mocks.close.mockClear();mocks.refreshHistory.mockClear();
});
afterEach(async () => { await act(async () => root.unmount()); vi.restoreAllMocks(); document.body.innerHTML=""; });
describe("File Library manual management workflow", () => {
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
    useFileManagementStore.setState({basket:tickets,clipboard:{action:"move",tickets},basketOpen:true});
    let resolve!: (value: any) => void;vi.mocked(api.executeTransfer).mockImplementation(() => new Promise((r)=>{resolve=r;}));
    await render();await click(`${t("fmPaste")} (2)`); await settle();
    await click(t("fmCancelRunning"));expect(tauriApi.cancelOperations).toHaveBeenCalledOnce();
    await act(async()=>{resolve({batch_id:"batch",logs:result(["success",remainingStatus])});});
    await settle();
    expect(useFileManagementStore.getState().basket.map((ticket)=>ticket.id)).toEqual(["two"]);
    expect(useFileManagementStore.getState().clipboard?.tickets.map((ticket)=>ticket.id)).toEqual(["two"]);
    expect(useFileManagementStore.getState().statuses.two).toContain(remainingStatus === "failed" ? "source_locked" : t("fmSkipped"));
  });
  it("reports stale sources nonmodally and keeps the persistent basket tickets", async()=>{
    useFileManagementStore.setState({basket:tickets,basketOpen:true});
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
    await click(t("fmSafeDelete"));

    expect(api.previewTransfer).toHaveBeenCalledWith(expect.objectContaining({
      ticketIds: ["one"], action: "trash", destinationDirectory: "C:/Source", skip: [], keepBoth: []
    }));
    expect(api.executeTransfer).toHaveBeenCalledWith({ previewId: "authority-trash", confirmed: true });
    expect(document.querySelector("[aria-modal=true]")).toBeNull();
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
      const value = (id: string): PaneContextValue => ({id,visible:true,focused:active===id,rootRef:{current:host},projection:defaultFileLibraryProjection,scope:{kind:"all"},setScope:vi.fn(),routingHint:id==="left"?"C:/Source":"C:/Target",updateLocation:vi.fn(),activate:()=>setActive(id),peerId:id==="left"?"right":"left",activatePeer:()=>{setActive(id==="left"?"right":"left");useFileManagementStore.getState().activatePane(id==="left"?"right":"left");},registerPreview:vi.fn(),beforeMutation,afterMutation});
      return <><PaneContext.Provider value={value("left")}><FileManagementToolbar/></PaneContext.Provider><PaneContext.Provider value={value("right")}><FileManagementToolbar/></PaneContext.Provider></>;
    }
    await act(async () => root.render(<TwoPanes/>));
    await act(async () => useFileManagementStore.getState().requestAction("copy_other"));
    expect(api.captureTransfer).toHaveBeenCalledWith({selection:source.selection});
    await settle();
    expect(api.previewTransfer).toHaveBeenCalledWith({ticketIds:["one","two"],action:"copy",destinationDirectory:"C:/Target",skip:[],keepBoth:[]});
    expect(api.executeTransfer).toHaveBeenCalledWith({previewId:"authority-preview",confirmed:true});
    expect(document.querySelector('[aria-modal="true"]')).toBeNull();
    expect(beforeMutation).toHaveBeenCalledOnce();
    expect(afterMutation).toHaveBeenCalledWith(["C:/Target","C:/Source","C:/Other"]);
    expect(clear).toHaveBeenCalledOnce();expect(clearTarget).not.toHaveBeenCalled();
    expect(useFileManagementStore.getState().transferOpen).toBe(false);
  });

  it("marks unavailable favorite locations and keeps them out of destination selection", async()=>{
    vi.mocked(api.fileLibraryPlaces).mockResolvedValue([{id:"offline",name:"Offline disk",routingHint:"Z:/Gone",favorite:true,current:false,visitedAt:1,unavailable:true}]);
    await render();await click(t("fmMore"));await click(t("fmPlaces"));expect(document.body.textContent).toContain(t("fmUnavailable"));
    const offline=[...document.querySelectorAll<HTMLButtonElement>("button")].find((b)=>b.textContent?.includes("Offline disk"))!;expect(offline.disabled).toBe(true);expect(mocks.navigate).not.toHaveBeenCalled();
  });
});
