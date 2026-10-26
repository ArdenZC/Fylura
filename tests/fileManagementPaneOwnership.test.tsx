// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
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
