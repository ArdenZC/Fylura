// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_APP_SETTINGS, createScanRootSetting } from "../src/hooks/useAppSettings";

const mocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  startManagedScan: vi.fn(),
  enqueueRoots: vi.fn(),
  getRuntimeCapabilities: vi.fn().mockResolvedValue(null),
  onGlobalHotkeyRegistrationFailed: vi.fn().mockResolvedValue(() => {}),
  refresh: vi.fn().mockResolvedValue(undefined),
  adoptConfiguredRootsIfScopeEmpty: vi.fn().mockReturnValue(false),
  setDefaultScanRoots: vi.fn(),
  initializeScanListeners: vi.fn(),
  initializeOperationQueue: vi.fn()
}));
vi.mock("../src/api/tauriApi", () => ({ tauriApi: mocks }));
vi.mock("../src/hooks/useAppChrome", () => ({ useAppChrome: () => ({ isSearchMode: false }) }));
vi.mock("../src/hooks/useFsWatcher", () => ({ useFsWatcher: () => {} }));
vi.mock("../src/hooks/useRulePersistence", () => ({ useRulePersistence: () => {} }));
vi.mock("../src/hooks/useWindowBehavior", () => ({ useWindowBehavior: () => ({}) }));
vi.mock("../src/store/useBackgroundIndexerStore", () => ({
  useBackgroundIndexerStore: (selector: (state: unknown) => unknown) => selector(mocks)
}));
vi.mock("../src/store/useFileLibraryStore", () => ({ useFileLibraryStore: { getState: () => mocks } }));
vi.mock("../src/store/useScanManagerStore", () => ({
  useScanManagerStore: Object.assign((selector: (state: unknown) => unknown) => selector(mocks), { getState: () => mocks })
}));
vi.mock("../src/store/useOperationQueueStore", () => ({
  useOperationQueueStore: (selector: (state: unknown) => unknown) => selector(mocks)
}));
import { AppRuntimeProviders } from "../src/components/AppRuntimeProviders";

let root: ReturnType<typeof createRoot> | undefined;
afterEach(() => { act(() => root?.unmount()); vi.clearAllMocks(); });

describe("Managed Library dormancy at app startup", () => {
  it.each([false, true])("opens Files without submitting Managed Scan or rewriting a saved profile (upgrade=%s)", async (upgrade) => {
    const settings = {
      ...DEFAULT_APP_SETTINGS,
      defaultScanFolders: upgrade ? [createScanRootSetting("C:/Users/Owner/Documents")] : [],
      customSearchRoots: upgrade ? [{ id: "search-root", path: "C:/Users/Owner/Downloads", enabled: true }] : [],
      backgroundIndexOnStartup: true
    };
    const original = JSON.stringify(settings);
    mocks.getSettings.mockResolvedValue({ settings, revision: 7 });
    const container = document.createElement("div");
    root = createRoot(container);
    await act(async () => {
      root?.render(createElement(AppRuntimeProviders, { children: createElement("p", null, "Files") }));
      await new Promise((resolve) => setTimeout(resolve, 30));
    });
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 30)); });
    expect(container.textContent).toBe("Files");
    expect(mocks.getSettings).toHaveBeenCalled();
    expect(mocks.setDefaultScanRoots).toHaveBeenCalledWith(settings.defaultScanFolders);
    expect(mocks.enqueueRoots).not.toHaveBeenCalled();
    expect(mocks.startManagedScan).not.toHaveBeenCalled();
    expect(mocks.saveSettings).not.toHaveBeenCalled();
    expect(JSON.stringify(settings)).toBe(original);
  });
});
