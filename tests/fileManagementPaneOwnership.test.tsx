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

it("rejects pointer drops onto folder rows and releases captured tickets", async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  const source = {
    signature: "source",
    count: 1,
    refs: [{ kind: "ephemeral" as const, browseSessionId: "session", entryId: "entry" }],
    visible: true,
    refresh: async () => undefined,
    clear: () => undefined,
  };
  const peer = {
    ...source,
    signature: "peer",
    refs: [],
  };
  useFileManagementStore.setState({
    surfaces: {
      source: { owner: "source", read: () => source },
      target: { owner: "target", read: () => peer },
    },
    activePaneId: "source",
    surface: () => source,
    basket: [],
    clipboard: null,
    transferOpen: false,
    pendingTransfer: null,
  });
  useAppStore.setState({ toast: null });
  const ticket = { id: "ticket", name: "file.txt", path: "E:/Source/file.txt", isDirectory: false };
  const capture = vi.spyOn(fileManagementApi, "captureTransfer").mockImplementation(async (request) =>
    request.releaseIds ? [] : [ticket],
  );
  const focus = vi.fn();
  let handlers!: ReturnType<typeof useWorkbenchDrag>;
  let sourcePaneElement!: HTMLElement;
  let peerPaneElement!: HTMLElement;
  let sourceRow!: HTMLElement;
  let folderRow!: HTMLElement;
  const sourcePane = { id: "source", path: "E:/Source", controller: { getState: () => ({ mode: "browse" }) } } as unknown as WorkbenchPane;
  const targetPane = { id: "target", path: "E:/Target", controller: { getState: () => ({ mode: "browse" }) } } as unknown as WorkbenchPane;
  const snapshot = {
    activeTabId: "tab",
    tabs: [{ id: "tab", dual: true, activePaneId: "source", panes: [sourcePane, targetPane] }],
  };
  const model = { getSnapshot: () => snapshot, subscribe: () => () => undefined } as unknown as FileWorkbenchModel;
  function Owner() {
    const workbenchRef = useRef<HTMLDivElement>(null);
    handlers = useWorkbenchDrag({
      workbenchRef,
      model,
      wide: true,
      dual: true,
      active: true,
      locked: false,
      focus,
      t: makeTranslator("en"),
    });
    return (
      <div
        ref={workbenchRef}
        onPointerMove={handlers.onPointerMove}
        onPointerUp={handlers.onPointerUp}
        onPointerCancel={handlers.onPointerCancel}
        onLostPointerCapture={handlers.onLostPointerCapture}
        onClickCapture={handlers.onClickCapture}
      >
        <div
          ref={(element) => { if (element) sourcePaneElement = element; }}
          data-pane-id="source"
          onPointerDown={(event) => handlers.onPointerDown(event, "tab", "source", "target", true)}
        >
          <div
            ref={(element) => { if (element) sourceRow = element; }}
            aria-selected="true"
            data-file-entry-kind="file"
          >
            source
          </div>
        </div>
        <div
          ref={(element) => { if (element) peerPaneElement = element; }}
          data-pane-id="target"
        >
          <main data-workspace-slot="content">
            <div
              ref={(element) => { if (element) folderRow = element; }}
              data-file-entry-kind="directory"
            >
              folder
            </div>
          </main>
        </div>
      </div>
    );
  }
  await act(async () => root.render(<Owner/>));
  const rect = [{ x: 0, y: 0, width: 100, height: 100, top: 0, right: 100, bottom: 100, left: 0, toJSON: () => ({}) }] as unknown as DOMRectList;
  sourcePaneElement.getClientRects = () => rect;
  peerPaneElement.getClientRects = () => rect;
  let captured = false;
  sourceRow.setPointerCapture = () => { captured = true; };
  sourceRow.hasPointerCapture = () => captured;
  sourceRow.releasePointerCapture = () => { captured = false; };
  vi.spyOn(document, "elementFromPoint").mockImplementation((x) => x > 100 ? folderRow : sourceRow);
  const pointerEvent = (type: string, x: number, y: number) => {
    const event = new Event(type, { bubbles: true, cancelable: true }) as Event & Record<string, unknown>;
    Object.assign(event, { button: 0, isPrimary: true, pointerId: 4, clientX: x, clientY: y, shiftKey: false });
    return event;
  };
  await act(async () => sourceRow.dispatchEvent(pointerEvent("pointerdown", 10, 10)));
  expect(capture).not.toHaveBeenCalled();
  await act(async () => {
    sourceRow.dispatchEvent(pointerEvent("pointermove", 20, 10));
    await Promise.resolve();
  });
  expect(capture).toHaveBeenCalledWith({
    refs: [{ kind: "ephemeral", browseSessionId: "session", entryId: "entry" }],
  });
  await act(async () => {
    sourceRow.dispatchEvent(pointerEvent("pointerup", 200, 20));
    await Promise.resolve();
    await Promise.resolve();
  });
  expect(focus).not.toHaveBeenCalled();
  expect(useFileManagementStore.getState().pendingTransfer).toBeNull();
  expect(capture).toHaveBeenCalledWith({ releaseIds: ["ticket"] });
  expect(useAppStore.getState().toast?.message).toContain("Dropping onto folder rows is not supported");
  capture.mockRestore();
  await act(async () => root.unmount());
  host.remove();
  useFileManagementStore.setState({ surfaces: {}, surface: null });
});
