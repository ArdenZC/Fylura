// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useWorkbenchDrag } from "../src/views/fileLibrary/workbench/useWorkbenchDrag";
import { fileManagementApi } from "../src/api/fileManagementApi";
import { makeTranslator } from "../src/i18n";
import type { WorkbenchPane } from "../src/views/fileLibrary/workbench/workbenchModel";
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

it("rejects a folder-row drop instead of silently targeting its parent pane, and releases the captured tickets", async () => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  const host=document.createElement("div");document.body.appendChild(host);const root=createRoot(host);
  const source={signature:"source",count:1,refs:[{kind:"ephemeral" as const,browseSessionId:"session",entryId:"entry"}],refresh:async()=>undefined,clear:()=>undefined};
  useFileManagementStore.setState({surfaces:{source:{owner:"source",read:()=>source}},activePaneId:"source",surface:()=>source,basket:[],clipboard:null,transferOpen:false,pendingTransfer:null});
  const ticket={id:"ticket",name:"file.txt",path:"C:/Source/file.txt",isDirectory:false};
  const capture=vi.spyOn(fileManagementApi,"captureTransfer").mockImplementation(async(request)=>request.releaseIds?[]:[ticket]);
  const focus=vi.fn();let handlers!:ReturnType<typeof useWorkbenchDrag>;
  function Owner(){handlers=useWorkbenchDrag({focus,locked:false,t:makeTranslator("en")});return null;}
  await act(async()=>root.render(<Owner/>));
  const row=document.createElement("div");row.setAttribute("aria-selected","true");row.setAttribute("draggable","true");
  const data=new Map<string,string>();const transfer={setData:(type:string,value:string)=>data.set(type,value),getData:(type:string)=>data.get(type)??"",effectAllowed:""};
  await handlers.startDrag({target:row,dataTransfer:transfer,preventDefault:vi.fn()} as any,{id:"source"} as WorkbenchPane);
  const folder=document.createElement("div");folder.setAttribute("data-file-entry-kind","directory");
  await handlers.drop({target:folder,dataTransfer:transfer,preventDefault:vi.fn(),shiftKey:false} as any,{id:"target",path:"C:/Target"} as WorkbenchPane,"tab");
  expect(focus).not.toHaveBeenCalled();expect(useFileManagementStore.getState().pendingTransfer).toBeNull();
  expect(capture).toHaveBeenCalledWith({releaseIds:["ticket"]});
  expect(useAppStore.getState().toast?.message).toContain("Dropping onto folder rows is not supported");
  handlers.endDrag();expect(capture).toHaveBeenCalledTimes(2);
  capture.mockRestore();await act(async()=>root.unmount());host.remove();
  useFileManagementStore.setState({surfaces:{},surface:null});
});
