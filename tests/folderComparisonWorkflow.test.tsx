// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { FolderComparison } from "../src/views/fileLibrary/compare/FolderComparison";
import { useFileManagementStore } from "../src/views/fileLibrary/management/fileManagementStore";
import type { BrowseSourceOwner } from "../src/views/fileLibrary/browse/browseSourceOwner";
import type { BrowsePresentationEntry } from "../src/views/fileLibrary/presentation/contracts";
const runtime = vi.hoisted(() => ({ platform: "darwin" as NodeJS.Platform }));
vi.mock("../src/contexts/AppContexts", async () => {
  const { makeTranslator } = await import("../src/i18n");
  return {
    useI18nContext: () => ({ t: makeTranslator("en"), language: "en" }),
    useChromeContext: () => ({ platform: runtime.platform }),
  };
});
const entry = (side: string, name: string): BrowsePresentationEntry => ({ source:"browse", displayName:name, entryKind:"file", size:10, modifiedAt:1,
  renderKey: name as BrowsePresentationEntry["renderKey"], entryRef:{kind:"ephemeral",browseSessionId:side,entryId:`${side}-${name}`} });
let root: Root, host: HTMLDivElement;
const sources: BrowseSourceOwner[] = [];
function publish() {
  useFileManagementStore.setState({ surfaces: Object.fromEntries(sources.map((s, i) => [String(i), {owner:String(i),read:()=>({signature:s.sessionId!,count:s.selectedCount,browseSource:s,refresh:s.refreshEnumeration,clear:s.clearSelection})}])) });
}
function source(side: string, names: string[]) {
  const result = { browse:{}, query:{text:null,entryKind:"all"}, entries:names.map((name)=>entry(side,name)), sessionId:side,
    enumerationState:"complete", selectedIds:new Set<string>(), selectedCount:0, skippedEntries:0, loadedCount:names.length,
    loadNextPage:vi.fn(async()=>{}), refreshEnumeration:vi.fn(async()=>{}), clearSelection:vi.fn(),
    selectIds: vi.fn((ids: string[]) => { Object.assign(result,{selectedIds:new Set(ids),selectedCount:ids.length}); publish(); }) } as unknown as BrowseSourceOwner;
  return result;
}
const click = async(text:string) => act(async()=>{[...host.querySelectorAll('button')].find((b)=>b.textContent?.trim()===text)!.click();});
beforeEach(async()=>{
  runtime.platform = "darwin";
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;
  sources.splice(0,sources.length,source("left",["same","left-only"]),source("right",["same","right-only"]));
  useFileManagementStore.setState({transferOpen:false,pendingAction:null}); publish();
  host=document.createElement('div');document.body.appendChild(host);root=createRoot(host);
  await act(async()=>root.render(<FolderComparison paneIds={["0","1"]} activate={(id)=>useFileManagementStore.getState().activatePane(id)}/>));
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();});
describe("comparison source-owned workflow",()=>{
  it("filters and selects the complete filtered side without overwriting the peer, then hands off the existing cross-pane action", async()=>{
    await click("Compare folders");
    await click("Visible only on left 1");
    await act(async()=>{host.querySelectorAll<HTMLButtonElement>('.u3-batch-actions button')[0].click();});
    expect(sources[0].selectIds).toHaveBeenCalledWith(["left-left-only"]);
    expect(sources[1].selectIds).not.toHaveBeenCalled();
    await act(async()=>{[...host.querySelectorAll<HTMLButtonElement>('.u3-batch-actions > div:first-child button')].find(b=>b.textContent==='Copy to other pane')!.click();});
    expect(useFileManagementStore.getState().activePaneId).toBe("0");
    expect(useFileManagementStore.getState().pendingAction).toBe("copy_other");
  });
  it("hides old results after a generation starts refreshing or fails, and never presents partial pages as a completed comparison", async()=>{
    await click("Compare folders");
    expect(host.querySelector('[role=table]')).not.toBeNull();
    await act(async()=>{Object.assign(sources[0],{enumerationState:"loading",entries:[]});publish();});
    expect(host.querySelector('[role=table]')).toBeNull();
    await act(async()=>{Object.assign(sources[0],{enumerationState:"failed"});publish();});
    expect(host.textContent).toContain('Enumeration failed');
    expect(host.querySelector('[role=table]')).toBeNull();
  });
  it("warns about skipped children, pending changes, filters and capacity rather than making full coverage claims", async()=>{
    await act(async()=>{Object.assign(sources[0],{skippedEntries:1});publish();});
    await click("Compare folders");
    expect(host.querySelector('[role=alert]')?.textContent).toContain('Enumeration coverage incomplete');
    await act(async()=>{Object.assign(sources[0],{pendingChange:{}});publish();});
    expect(host.textContent).toContain('Folder changed');
    expect(host.querySelector('[role=table]')).toBeNull();
    await act(async()=>{Object.assign(sources[0],{pendingChange:null,query:{text:'sample',entryKind:'all'}});publish();});
    expect(host.textContent).toContain('Clear name and type filters');
    await act(async()=>{Object.assign(sources[0],{query:{text:null,entryKind:'all'},entries:Array(10001).fill(entry('left','x'))});publish();});
    expect(host.textContent).toContain('Comparison capacity exceeded');
    expect(host.querySelector('[role=table]')).toBeNull();
  });
  it("does not read additional pages until comparison is opened and stops on pause", async()=>{
    await act(async()=>{Object.assign(sources[0],{enumerationState:'partial'});publish();});
    expect(sources[0].loadNextPage).not.toHaveBeenCalled();
    await click('Compare folders');
    await act(async()=>{await new Promise(resolve=>setTimeout(resolve,10));});
    expect(sources[0].loadNextPage).toHaveBeenCalledTimes(1);
    await click('Pause');
    await act(async()=>{publish();await new Promise(resolve=>setTimeout(resolve,10));});
    expect(sources[0].loadNextPage).toHaveBeenCalledTimes(1);
  });
  it("keeps directory Copy available on Windows and disables only directory Move", async()=>{
    runtime.platform = "win32";
    const directory = { ...entry("left", "folder"), entryKind: "directory" as const };
    Object.assign(sources[0], {
      entries: [directory],
      selectedIds: new Set([directory.entryRef.entryId]),
      selectedCount: 1,
    });
    publish();
    await act(async()=>root.render(<FolderComparison paneIds={["0","1"]} activate={(id)=>useFileManagementStore.getState().activatePane(id)}/>));
    await click("Compare folders");
    const buttons = [...host.querySelectorAll<HTMLButtonElement>(".u3-batch-actions > div:first-child button")];
    expect(buttons.find((button) => button.textContent === "Add to basket")?.disabled).toBe(false);
    expect(buttons.find((button) => button.textContent === "Copy to other pane")?.disabled).toBe(false);
    expect(buttons.find((button) => button.textContent === "Move to other pane")?.disabled).toBe(true);
    await act(async()=>buttons.find((button) => button.textContent === "Copy to other pane")!.click());
    expect(useFileManagementStore.getState().pendingAction).toBe("copy_other");
  });
});
