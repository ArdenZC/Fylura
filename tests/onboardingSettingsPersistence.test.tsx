// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ChromeProvider,
  SettingsProvider,
  type ChromeContextValue,
  type SettingsContextValue
} from "../src/contexts/AppContexts";
import { makeTranslator } from "../src/i18n";
import { OnboardingDialog, ONBOARDING_STORAGE_KEY } from "../src/components/OnboardingDialog";
import { DEFAULT_APP_SETTINGS, createScanRootSetting } from "../src/hooks/useAppSettings";
import type { AppSettings } from "../src/types/domain";

const apiMocks = vi.hoisted(() => ({
  getAIFeatureReadiness: vi.fn(),
  listGlobalIndexSources: vi.fn(),
  authorizeGlobalIndexSource: vi.fn(),
  setGlobalIndexSourceEnabled: vi.fn(),
  startGlobalIndex: vi.fn(),
  rebuildGlobalIndexSource: vi.fn(),
  addManagedScope: vi.fn(),
  saveSettings: vi.fn(),
  getSettings: vi.fn(),
  startManagedScan: vi.fn(),
  onError: vi.fn()
}));

vi.mock("../src/api/tauriApi", () => ({ tauriApi: apiMocks }));

const t = makeTranslator("en");

function readinessFixture() {
  const provider = {
    state: "disabled" as const,
    reason: "provider_disabled",
    providerMode: "local" as const,
    providerKind: "local_model",
    providerPreset: null,
    model: null,
    credentialRequired: false,
    credentialConfigured: false,
    settingsRevision: "test",
    bindingFingerprint: "test"
  };
  return {
    provider,
    managedScopes: [],
    cleanup: {
      state: "disabled" as const,
      reason: "provider_disabled",
      provider,
      cleanupAiEnabled: false,
      localAiAllowed: false,
      cloudAiAllowed: false,
      bindingFingerprint: "test",
      disclosure: {
        providerPayloadIncludesFileName: false,
        providerPayloadIncludesParentPath: false,
        providerPayloadIncludesFullPath: false,
        providerPayloadIncludesFileContent: false,
        providerContentIsBounded: true
      }
    }
  };
}

const existingRoot = {
  ...createScanRootSetting("C:\\Users\\Owner\\Documents", "2026-10-08T00:00:00.000Z"),
  id: "backend-owned-root-id",
  label: "Documents"
};

const existingProfile: AppSettings = {
  ...DEFAULT_APP_SETTINGS,
  defaultScanFolders: [existingRoot],
  backgroundIndexOnStartup: true
};

const cleanProfile: AppSettings = {
  ...DEFAULT_APP_SETTINGS,
  defaultScanFolders: [],
  backgroundIndexOnStartup: false
};

describe("first-run intro persistence boundary", () => {
  let root: Root;
  let setView: ReturnType<typeof vi.fn<(view: string) => void>>;
  let setters: {
    updateSettings: ReturnType<typeof vi.fn>;
    setDefaultScanFolders: ReturnType<typeof vi.fn>;
    setBackgroundIndexOnStartup: ReturnType<typeof vi.fn>;
  };
  const nativeGetClientRects = HTMLElement.prototype.getClientRects;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    localStorage.clear();
    sessionStorage.clear();
    document.body.innerHTML = '<div id="app-shell-content"><button id="background">Background</button></div><div id="test-root"></div>';
    HTMLElement.prototype.getClientRects = () => [{ width: 120, height: 40, top: 0, left: 0, right: 120, bottom: 40, x: 0, y: 0, toJSON() { return {}; } }] as unknown as DOMRectList;
    setView = vi.fn();
    setters = {
      updateSettings: vi.fn().mockResolvedValue(true),
      setDefaultScanFolders: vi.fn().mockResolvedValue(true),
      setBackgroundIndexOnStartup: vi.fn().mockResolvedValue(true)
    };
    apiMocks.getAIFeatureReadiness.mockReset().mockResolvedValue(readinessFixture());
    apiMocks.listGlobalIndexSources.mockReset().mockResolvedValue([]);
    apiMocks.authorizeGlobalIndexSource.mockReset().mockResolvedValue(undefined);
    apiMocks.setGlobalIndexSourceEnabled.mockReset().mockResolvedValue(undefined);
    apiMocks.startGlobalIndex.mockReset().mockResolvedValue(undefined);
    apiMocks.rebuildGlobalIndexSource.mockReset().mockResolvedValue(undefined);
    apiMocks.addManagedScope.mockReset().mockResolvedValue({ id: "managed-scope" });
    apiMocks.saveSettings.mockReset().mockResolvedValue({ settings: cleanProfile, revision: 1 });
    apiMocks.getSettings.mockReset().mockResolvedValue({ settings: cleanProfile, revision: 0 });
    apiMocks.startManagedScan.mockReset().mockResolvedValue(undefined);
    apiMocks.onError.mockReset();
    root = createRoot(document.getElementById("test-root")!);
  });

  afterEach(() => {
    act(() => root.unmount());
    HTMLElement.prototype.getClientRects = nativeGetClientRects;
    document.body.innerHTML = "";
    localStorage.clear();
  });

  function renderIntro(settings: AppSettings) {
    const settingsValue = {
      settings,
      isLoadingSettings: false,
      settingsError: "",
      updateSettings: setters.updateSettings,
      setFolderNamingLanguage: vi.fn().mockResolvedValue(true),
      setDefaultScanFolders: setters.setDefaultScanFolders,
      setRestoreRetentionDays: vi.fn().mockResolvedValue(true),
      setLaunchAtLogin: vi.fn().mockResolvedValue(true),
      setBackgroundIndexOnStartup: setters.setBackgroundIndexOnStartup,
      setSearchHotkey: vi.fn().mockResolvedValue(true),
      setSearchScopeMode: vi.fn().mockResolvedValue(true),
      setCustomSearchRoots: vi.fn().mockResolvedValue(true),
      setOrganizeRootMode: vi.fn().mockResolvedValue(true),
      setOrganizeRootPath: vi.fn().mockResolvedValue(true)
    } as unknown as SettingsContextValue;
    const chromeValue = {
      language: "en",
      setLanguage: vi.fn(),
      theme: "light",
      setTheme: vi.fn(),
      view: "library",
      setView,
      onError: apiMocks.onError,
      t
    } as unknown as ChromeContextValue;
    act(() => root.render(createElement(ChromeProvider, {
      value: chromeValue,
      children: createElement(SettingsProvider, { value: settingsValue, children: createElement(OnboardingDialog) })
    })));
  }

  async function flushAsync() {
    await act(async () => {
      await Promise.resolve();
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      await Promise.resolve();
    });
  }

  async function click(selector: string) {
    const button = document.querySelector<HTMLButtonElement>(selector);
    expect(button, selector).toBeTruthy();
    await act(async () => button?.click());
    await flushAsync();
  }

  function expectNoPersistenceOrScan() {
    expect(apiMocks.saveSettings).not.toHaveBeenCalled();
    expect(apiMocks.addManagedScope).not.toHaveBeenCalled();
    expect(apiMocks.startManagedScan).not.toHaveBeenCalled();
    expect(apiMocks.authorizeGlobalIndexSource).not.toHaveBeenCalled();
    expect(apiMocks.setGlobalIndexSourceEnabled).not.toHaveBeenCalled();
    expect(apiMocks.startGlobalIndex).not.toHaveBeenCalled();
    expect(apiMocks.rebuildGlobalIndexSource).not.toHaveBeenCalled();
    expect(setters.updateSettings).not.toHaveBeenCalled();
    expect(setters.setDefaultScanFolders).not.toHaveBeenCalled();
    expect(setters.setBackgroundIndexOnStartup).not.toHaveBeenCalled();
  }

  it("never writes settings, scan roots, managed scopes or index authorization when the intro is skipped", async () => {
    renderIntro(cleanProfile);
    await flushAsync();
    await click("[data-onboarding-skip]");

    expectNoPersistenceOrScan();
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(setView).toHaveBeenCalledWith("library");
  });

  it("never writes settings when the user starts from page one on a clean profile", async () => {
    renderIntro(cleanProfile);
    await flushAsync();
    await click("[data-onboarding-start]");

    expectNoPersistenceOrScan();
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(setView).toHaveBeenCalledWith("library");
  });

  it("preserves an existing Managed Library profile without rewriting its roots", async () => {
    const before = JSON.stringify(existingProfile);
    renderIntro(existingProfile);
    await flushAsync();
    await click("[data-onboarding-next]");
    await click("[data-onboarding-next]");
    await click("[data-onboarding-next]");
    await click("[data-onboarding-finish]");

    expectNoPersistenceOrScan();
    // The upgrade path keeps the configured roots and startup preference untouched.
    expect(JSON.stringify(existingProfile)).toBe(before);
    expect(existingProfile.defaultScanFolders).toHaveLength(1);
    expect(existingProfile.backgroundIndexOnStartup).toBe(true);
  });

  it("keeps the completion flag across a clean-profile restart", async () => {
    renderIntro(cleanProfile);
    await flushAsync();
    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    await click("[data-onboarding-skip]");

    act(() => root.unmount());
    document.body.innerHTML = '<div id="app-shell-content"></div><div id="test-root"></div>';
    root = createRoot(document.getElementById("test-root")!);
    renderIntro(cleanProfile);
    await flushAsync();

    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(apiMocks.getAIFeatureReadiness).toHaveBeenCalledTimes(1);
    expectNoPersistenceOrScan();
  });

  it("stays a no-op when the completion flag is already present", async () => {
    localStorage.setItem(ONBOARDING_STORAGE_KEY, "true");
    renderIntro(existingProfile);
    await flushAsync();

    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(apiMocks.getAIFeatureReadiness).not.toHaveBeenCalled();
    expect(apiMocks.listGlobalIndexSources).not.toHaveBeenCalled();
    expectNoPersistenceOrScan();
  });
});
