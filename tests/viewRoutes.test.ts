import { afterEach, describe, expect, it } from "vitest";
import type { View } from "../src/types/ui";
import { useAppStore } from "../src/store/useAppStore";
import { initialViewFromSearch, normalizeViewInput } from "../src/utils/viewRoutes";

describe("canonical product view routes", () => {
  afterEach(() => useAppStore.setState({ view: "library", automationSurface: "intents" }));

  it("accepts canonical Automation and normalizes only the legacy Rules route", () => {
    expect(normalizeViewInput("automation")).toBe("automation");
    expect(normalizeViewInput("rules")).toBe("automation");
    expect(normalizeViewInput("unknown-view")).toBeNull();
    expect(normalizeViewInput(null)).toBeNull();
  });

  it("keeps legacy and canonical deep links on the Intents default and falls back safely", () => {
    expect(initialViewFromSearch("?view=automation")).toBe("automation");
    expect(initialViewFromSearch("?view=rules")).toBe("automation");
    expect(initialViewFromSearch("?view=unknown-view")).toBe("library");
    expect(initialViewFromSearch("")).toBe("library");
  });

  it("stores only canonical routes and resets Advanced Policies on every ordinary Automation entry", () => {
    useAppStore.getState().setAutomationSurface("advanced-policies");
    useAppStore.getState().setView("automation");
    expect(useAppStore.getState().view).toBe("automation");
    expect(useAppStore.getState().automationSurface).toBe("intents");

    useAppStore.getState().setAutomationSurface("advanced-policies");
    useAppStore.getState().setView("rules" as unknown as View);
    expect(useAppStore.getState().view).toBe("automation");
    expect(useAppStore.getState().automationSurface).toBe("intents");

    useAppStore.getState().setView("unknown-view" as View);
    expect(useAppStore.getState().view).toBe("library");
  });
});
