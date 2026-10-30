import { describe, expect, it } from "vitest";
import { nativeDropPaths, physicalDropPointToClient } from "./useNativeFileDrop";

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
});
