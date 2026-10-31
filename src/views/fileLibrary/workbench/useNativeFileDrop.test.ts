import { describe, expect, it } from "vitest";
import { NativeDropRequestGate, nativeDropPaths, physicalDropPointToClient, sameExternalDropTarget } from "./useNativeFileDrop";

describe("native file drop coordinate and event routing", () => {
  it("converts Tauri physical pixels to WebView client coordinates at DPI and zoom scales", () => {
    expect(physicalDropPointToClient({ x: 1536, y: 900 }, 1.5)).toEqual({
      x: 1024,
      y: 600,
    });
    expect(physicalDropPointToClient({ x: 800, y: 400 }, 1)).toEqual({
      x: 800,
      y: 400,
    });
    expect(physicalDropPointToClient({ x: 1600, y: 800 }, 2)).toEqual({
      x: 800,
      y: 400,
    });
    expect(physicalDropPointToClient({ x: 20, y: 10 }, 0)).toEqual({
      x: 20,
      y: 10,
    });
  });

  it("admits candidate paths only from a native drop payload", () => {
    expect(nativeDropPaths({ type: "enter", paths: ["E:/fixture/file.txt"] })).toBeNull();
    expect(nativeDropPaths({ type: "drop", paths: [] })).toBeNull();
    expect(nativeDropPaths({ type: "drop", paths: ["E:/fixture/file.txt"] })).toEqual([
      "E:/fixture/file.txt",
    ]);
  });

  it("serializes captures and keeps a cancelled asynchronous request as the active owner until it settles", () => {
    const gate = new NativeDropRequestGate();
    const first = gate.begin();
    expect(first).not.toBeNull();
    expect(gate.begin()).toBeNull();
    gate.cancel(first!);
    expect(gate.isCurrent(first!)).toBe(false);
    expect(gate.begin()).toBeNull();
    gate.finish(first!);

    const second = gate.begin();
    expect(second).not.toBeNull();
    gate.finish(first!);
    expect(gate.isCurrent(second!)).toBe(true);
    gate.finish(second!);
    expect(gate.begin()).not.toBeNull();
  });

  it("requires the same active tab, pane, directory, and folder row after capture", () => {
    const target = {
      tabId: "tab-a",
      paneId: "pane-right",
      destination: "",
      destinationEntry: { kind: "ephemeral" as const, browseSessionId: "session-a", entryId: "folder-a" },
      label: "target",
    };
    expect(sameExternalDropTarget(target, { ...target })).toBe(true);
    expect(sameExternalDropTarget(target, { ...target, tabId: "tab-b" })).toBe(false);
    expect(sameExternalDropTarget(target, { ...target, paneId: "pane-left" })).toBe(false);
    expect(sameExternalDropTarget(target, { ...target, destinationEntry: { ...target.destinationEntry, entryId: "folder-b" } })).toBe(false);
    expect(sameExternalDropTarget(target, null)).toBe(false);
  });
});
