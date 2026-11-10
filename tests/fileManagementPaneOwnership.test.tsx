// @vitest-environment happy-dom
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useWorkbenchDrag } from "../src/views/fileLibrary/workbench/useWorkbenchDrag";
import { fileManagementApi } from "../src/api/fileManagementApi";
import { makeTranslator } from "../src/i18n";
import type { FileWorkbenchModel, WorkbenchPane } from "../src/views/fileLibrary/workbench/workbenchModel";
import { useAppStore } from "../src/store/useAppStore";
import { PaneContext, type PaneContextValue } from "../src/views/fileLibrary/workbench/PaneContext";
import { useFileManagementStore, useFileManagementSurface } from "../src/views/fileLibrary/management/fileManagementStore";

it("keeps the active source registered throughout selection changes, then releases it on real unmount", async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  const host=document.createElement("div");document.body.appendChild(host);const root=createRoot(host);
  useFileManagementStore.setState({surfaces:{},activePaneId:"left",owner:null,surface:null});
  function Source() {
    const [count,setCount]=useState(1);
    useFileManagementSurface({signature:`selection-${count}`,count,refs:[],refresh:async()=>undefined,clear:()=>undefined});
    return <button onClick={()=>setCount(count+1)}>Select another file</button>;
  }
  const pane={id:"left",visible:true,focused:true,routingHint:"C:/Source"} as PaneContextValue;
  await act(async()=>root.render(<PaneContext.Provider value={pane}><Source/></PaneContext.Provider>));
  const missing:boolean[]=[];
  const stop=useFileManagementStore.subscribe((state)=>missing.push(state.surface===null));
  await act(async()=>host.querySelector("button")!.click());
  expect(useFileManagementStore.getState().surface?.().count).toBe(2);
  expect(missing).not.toContain(true);
  stop();await act(async()=>root.unmount());
  expect(useFileManagementStore.getState().surfaces.left).toBeUndefined();
  expect(useFileManagementStore.getState().surface).toBeNull();host.remove();
});

it.each([
  { name: "single-pane folder", single: true, samePane: true, valid: true, stale: false, shift: false },
  { name: "dual-pane peer folder", single: false, samePane: false, valid: true, stale: false, shift: false },
  { name: "dual-pane source folder with Shift", single: false, samePane: true, valid: true, stale: false, shift: true },
  { name: "folder without opaque ref", single: false, samePane: false, valid: false, stale: false, shift: false },
  { name: "changed source selection", single: true, samePane: true, valid: true, stale: true, shift: false },
  { name: "changed target directory", single: false, samePane: false, valid: true, stale: false, shift: false, staleTarget: true },
  { name: "cancelled gesture", single: false, samePane: false, valid: true, stale: false, shift: false, cancelled: true },
  ...Array.from({ length: 4 }, (_, from) => Array.from({ length: 4 }, (_, to) => ({
    name: `four-pane ${from + 1}→${to + 1}`, single: false, samePane: false,
    valid: true, stale: false, shift: to % 2 === 1, from, to,
  })).filter((item) => item.from !== item.to)).flat(),
])("routes $name through the existing guarded transfer owner", async (scenario) => {
  const { single, samePane, valid, stale, shift } = scenario;
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"); document.body.appendChild(host);
  const root = createRoot(host);
  const source = { signature: "source", count: 1,
    refs: [{ kind: "ephemeral" as const, browseSessionId: "session", entryId: "entry" }],
    browse: { sessionId: "session", pathRef: { id: "root" } },
    visible: true, refresh: async () => undefined, clear: () => undefined };
  const peer = { ...source, signature: "peer", refs: [], browse: { sessionId: "peer-session", pathRef: { id: "peer-root" } } };
  useFileManagementStore.setState({ surfaces: {
    source: { owner: "source", read: () => source }, target: { owner: "target", read: () => peer },
  }, activePaneId: "source", surface: () => source, basket: [], clipboard: null, transferOpen: false, transferActive: false, pendingTransfer: null });
  useAppStore.setState({ toast: null });
  const ticket = { id: "ticket", name: "file.txt", path: "E:/Source/file.txt", isDirectory: false };
  const capture = vi.spyOn(fileManagementApi, "captureTransfer").mockImplementation(async (request) => request.releaseIds ? [] : [ticket]);
  const focus = vi.fn();
  let handlers!: ReturnType<typeof useWorkbenchDrag>;
  let sourcePaneElement!: HTMLElement, peerPaneElement!: HTMLElement, sourceRow!: HTMLElement, folderRow!: HTMLElement;
  const sourcePane = { id: "source", path: "E:/Source", controller: { getState: () => ({ mode: "browse" }) } } as unknown as WorkbenchPane;
  const targetPane = { id: "target", path: "E:/Target", controller: { getState: () => ({ mode: "browse" }) } } as unknown as WorkbenchPane;
  const panes = "from" in scenario
    ? Array.from({ length: 4 }, (_, i) => i === scenario.from ? sourcePane : i === scenario.to ? targetPane : { ...targetPane, id: `other-${i}` })
    : [sourcePane, targetPane];
  const snapshot = { activeTabId: "tab", tabs: [{ id: "tab", paneCount: single ? 1 : panes.length, activePaneId: "source", panes }] };
  const model = { getSnapshot: () => snapshot, subscribe: () => () => undefined } as unknown as FileWorkbenchModel;
  function Owner() {
    const workbenchRef = useRef<HTMLDivElement>(null);
    handlers = useWorkbenchDrag({ workbenchRef, model, wide: !single, active: true, locked: false, focus, t: makeTranslator("en") });
    const folder = <div ref={(el) => { if (el) folderRow = el; }} data-file-entry-kind="directory"
      data-browse-session-id={valid ? samePane ? "session" : "peer-session" : undefined} data-browse-entry-id={valid ? "folder" : undefined}>folder</div>;
    return <div ref={workbenchRef} onPointerMove={handlers.onPointerMove} onPointerUp={handlers.onPointerUp}
      onPointerCancel={handlers.onPointerCancel} onLostPointerCapture={handlers.onLostPointerCapture} onClickCapture={handlers.onClickCapture}>
      <div ref={(el) => { if (el) sourcePaneElement = el; }} data-pane-id="source"
        onPointerDown={(event) => handlers.onPointerDown(event, "tab", "source", true)}>
        <main data-workspace-slot="content">
          <div ref={(el) => { if (el) sourceRow = el; }} aria-selected="true" data-file-entry-kind="file">source</div>
          {samePane ? folder : null}
        </main>
      </div>
      <div ref={(el) => { if (el) peerPaneElement = el; }} data-pane-id="target" hidden={single}>
        <main data-workspace-slot="content">{samePane ? null : folder}</main>
      </div>
    </div>;
  }
  await act(async () => root.render(<Owner/>));
  const rect = [{ x: 0, y: 0, width: 100, height: 100, top: 0, right: 100, bottom: 100, left: 0, toJSON: () => ({}) }] as unknown as DOMRectList;
  sourcePaneElement.getClientRects = () => rect; peerPaneElement.getClientRects = () => rect;
  let captured = false;
  sourceRow.setPointerCapture = () => { captured = true; }; sourceRow.hasPointerCapture = () => captured; sourceRow.releasePointerCapture = () => { captured = false; };
  const hit = vi.spyOn(document, "elementFromPoint").mockImplementation((x) => x > 100 ? folderRow : sourceRow);
  const pointerEvent = (type: string, x: number) => {
    const event = new Event(type, { bubbles: true, cancelable: true }) as Event & Record<string, unknown>;
    Object.assign(event, { button: 0, isPrimary: true, pointerId: 4, clientX: x, clientY: 10, shiftKey: shift }); return event;
  };
  await act(async () => sourceRow.dispatchEvent(pointerEvent("pointerdown", 10)));
  expect(capture).not.toHaveBeenCalled();
  await act(async () => { sourceRow.dispatchEvent(pointerEvent("pointermove", 20)); await Promise.resolve(); });
  expect(capture).toHaveBeenCalledWith({ refs: [{ kind: "ephemeral", browseSessionId: "session", entryId: "entry" }] });
  if ("staleTarget" in scenario) targetPane.path = "E:/ChangedTarget";
  if ("cancelled" in scenario) await act(async () => sourceRow.dispatchEvent(pointerEvent("pointercancel", 200)));
  if (stale) source.refs = [{ kind: "ephemeral", browseSessionId: "session", entryId: "changed" }];
  await act(async () => { sourceRow.dispatchEvent(pointerEvent("pointerup", 200)); await Promise.resolve(); await Promise.resolve(); });
  if (valid && !stale && !("staleTarget" in scenario) && !("cancelled" in scenario)) {
    const paneId = samePane ? "source" : "target";
    expect(focus).toHaveBeenCalledWith("tab", paneId);
    expect(useFileManagementStore.getState().pendingTransfer).toMatchObject({ paneId, sourcePaneId: "source", tickets: [ticket], action: shift ? "move" : "copy", destination: "",
      destinationEntry: { kind: "ephemeral", browseSessionId: samePane ? "session" : "peer-session", entryId: "folder" } });
    expect(capture).not.toHaveBeenCalledWith({ releaseIds: ["ticket"] });
  } else {
    expect(focus).not.toHaveBeenCalled(); expect(useFileManagementStore.getState().pendingTransfer).toBeNull();
    expect(capture).toHaveBeenCalledWith({ releaseIds: ["ticket"] });
  }
  hit.mockRestore(); capture.mockRestore(); await act(async () => root.unmount()); host.remove();
  useFileManagementStore.setState({ surfaces: {}, surface: null, pendingTransfer: null });
});
