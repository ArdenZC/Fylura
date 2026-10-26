// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => { localStorage.removeItem("zc-sidebar-hidden"); vi.restoreAllMocks(); vi.resetModules(); });

describe("main sidebar preference", () => {
  it("starts expanded, retains the explicit choice across routes and restores it on restart", async () => {
    localStorage.removeItem("zc-sidebar-hidden");
    const { useAppStore } = await import("../src/store/useAppStore");
    expect(useAppStore.getState().sidebarHidden).toBe(false);
    useAppStore.getState().setSidebarHidden(true);
    for (const view of ["library", "settings", "scanner"] as const) {
      useAppStore.getState().setView(view);
      expect(useAppStore.getState().sidebarHidden).toBe(true);
    }
    expect(localStorage.getItem("zc-sidebar-hidden")).toBe("true");
    vi.resetModules();
    const restarted = (await import("../src/store/useAppStore")).useAppStore;
    expect(restarted.getState().sidebarHidden).toBe(true);
    restarted.getState().setSidebarHidden(false);
    expect(localStorage.getItem("zc-sidebar-hidden")).toBe("false");
  });

  it("keeps the toggle usable when optional preference storage fails", async () => {
    const { useAppStore } = await import("../src/store/useAppStore");
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("storage unavailable"); });
    expect(() => useAppStore.getState().setSidebarHidden(true)).not.toThrow();
    expect(useAppStore.getState().sidebarHidden).toBe(true);
    useAppStore.getState().setSidebarHidden(false);
    expect(useAppStore.getState().sidebarHidden).toBe(false);
  });
});
