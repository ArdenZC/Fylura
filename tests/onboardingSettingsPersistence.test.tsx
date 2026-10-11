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
import { OnboardingDialog } from "../src/components/OnboardingDialog";
import { makeTranslator } from "../src/i18n";
import {
  createScanRootSetting,
  DEFAULT_APP_SETTINGS,
  defaultScanRootSettingsEqual,
  useAppSettings
} from "../src/hooks/useAppSettings";
import { ONBOARDING_STORAGE_KEY } from "../src/components/OnboardingDialog";
import type { SaveSettingsRequest, VersionedAppSettings } from "../src/types/domain";

const apiMocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  getAIFeatureReadiness: vi.fn(),
  addManagedScope: vi.fn(),
  onError: vi.fn()
}));
const dialogMocks = vi.hoisted(() => ({ open: vi.fn() }));

vi.mock("../src/api/tauriApi", () => ({
  tauriApi: {
    getSettings: apiMocks.getSettings,
    saveSettings: apiMocks.saveSettings,
    getAIFeatureReadiness: apiMocks.getAIFeatureReadiness,
    addManagedScope: apiMocks.addManagedScope
  }
}));
vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);

function PersistedOnboardingHarness({ setView }: { setView: (view: string) => void }) {
  const state = useAppSettings({
    isDatabaseReady: true,
    onError: apiMocks.onError,
    formatSaveError: (error) => String(error)
  });
  const setDefaultScanFolders = async (next: VersionedAppSettings["settings"]["defaultScanFolders"]) => {
    const result = await state.updateSettingsWithResult({ defaultScanFolders: next });
    return result.persisted && defaultScanRootSettingsEqual(result.settings.defaultScanFolders, next);
  };
  const unusedSetter = async () => true;
  const settingsValue = {
    ...state,
    settingsError: "",
    setFolderNamingLanguage: unusedSetter,
    setDefaultScanFolders,
    setRestoreRetentionDays: unusedSetter,
    setLaunchAtLogin: unusedSetter,
    setBackgroundIndexOnStartup: unusedSetter,
    setSearchHotkey: unusedSetter,
    setSearchScopeMode: unusedSetter,
    setCustomSearchRoots: unusedSetter,
    setOrganizeRootMode: unusedSetter,
    setOrganizeRootPath: unusedSetter
  } as unknown as SettingsContextValue;
  const chromeValue = {
    language: "en",
    setLanguage: vi.fn(),
    theme: "light",
    setTheme: vi.fn(),
    view: "scanner",
    setView,
    onError: vi.fn(),
    t: makeTranslator("en")
  } as unknown as ChromeContextValue;

  return createElement(ChromeProvider, {
    value: chromeValue,
    children: createElement(SettingsProvider, {
      value: settingsValue,
      children: createElement(OnboardingDialog)
    })
  });
}

describe("Onboarding settings persistence boundary", () => {
  let root: Root;
  let setView: ReturnType<typeof vi.fn<(view: string) => void>>;
  const nativeGetClientRects = HTMLElement.prototype.getClientRects;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    localStorage.clear();
    document.body.innerHTML = '<div id="app-shell-content"><button id="background">Background</button></div><div id="test-root"></div>';
    HTMLElement.prototype.getClientRects = () => [{ width: 120, height: 40, top: 0, left: 0, right: 120, bottom: 40, x: 0, y: 0, toJSON() { return {}; } }] as unknown as DOMRectList;
    setView = vi.fn();
    apiMocks.getSettings.mockReset().mockResolvedValue({ settings: DEFAULT_APP_SETTINGS, revision: 0 });
    apiMocks.saveSettings.mockReset();
    apiMocks.getAIFeatureReadiness.mockReset().mockResolvedValue({
      provider: {
        state: "disabled",
        reason: "provider_disabled",
        providerMode: "local",
        providerKind: "local_model",
        providerPreset: null,
        model: null,
        credentialRequired: false,
        credentialConfigured: false,
        settingsRevision: "test",
        bindingFingerprint: "test"
      },
      managedScopes: [],
      cleanup: {
        state: "disabled",
        reason: "provider_disabled",
        provider: {
          state: "disabled",
          reason: "provider_disabled",
          providerMode: "local",
          providerKind: "local_model",
          providerPreset: null,
          model: null,
          credentialRequired: false,
          credentialConfigured: false,
          settingsRevision: "test",
          bindingFingerprint: "test"
        },
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
    });
    apiMocks.addManagedScope.mockReset().mockResolvedValue({ id: "managed-scope" });
    apiMocks.onError.mockReset();
    dialogMocks.open.mockReset().mockResolvedValue("C:\\OwnerQualification\\fixture");
    root = createRoot(document.getElementById("test-root")!);
  });

  afterEach(() => {
    act(() => root.unmount());
    HTMLElement.prototype.getClientRects = nativeGetClientRects;
    document.body.innerHTML = "";
    localStorage.clear();
  });

  function renderOnboarding() {
    act(() => root.render(createElement(PersistedOnboardingHarness, { setView })));
  }

  async function flushAsync() {
    await act(async () => {
      await Promise.resolve();
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      await Promise.resolve();
    });
  }

  async function clickNext() {
    const next = document.querySelector<HTMLButtonElement>("[data-onboarding-next]");
    expect(next).toBeTruthy();
    await act(async () => next?.click());
    await flushAsync();
  }

  async function selectFolderAtOnboardingStep() {
    await clickNext();
    await act(async () => document.querySelector<HTMLButtonElement>("[data-onboarding-skip-ai]")?.click());
    const chooseFolder = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent?.includes("Choose folder"));
    expect(chooseFolder).toBeTruthy();
    await act(async () => chooseFolder?.click());
    await flushAsync();
    expect(dialogMocks.open).toHaveBeenCalledWith(expect.objectContaining({ directory: true, multiple: false }));
  }

  it("continues after the persisted root is returned in backend-normalized form", async () => {
    apiMocks.saveSettings.mockImplementation(async (request: SaveSettingsRequest) => {
      const root = request.settings.defaultScanFolders[0];
      return {
        settings: {
          ...request.settings,
          defaultScanFolders: [{
            ...root,
            id: "backend-owned-root-id",
            path: "c:/ownerqualification/fixture/",
            label: "Fixture",
            createdAt: "2026-10-08T00:00:01.000Z"
          }]
        },
        revision: request.expectedRevision + 1
      };
    });
    renderOnboarding();
    await flushAsync();
    await selectFolderAtOnboardingStep();

    await clickNext();

    expect(apiMocks.saveSettings).toHaveBeenCalledOnce();
    expect(document.querySelector('[data-onboarding-step="5"]')).toBeTruthy();
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBeNull();
    await clickNext();
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(setView).toHaveBeenCalledWith("library");
  });

  it("keeps Onboarding incomplete after a backend side-effect rollback", async () => {
    const backendError = "settings_save_failure:watcher_runtime_failure";
    apiMocks.saveSettings.mockRejectedValue(new Error(backendError));
    renderOnboarding();
    await flushAsync();
    await selectFolderAtOnboardingStep();

    await clickNext();

    expect(apiMocks.getSettings).toHaveBeenCalledTimes(2);
    expect(apiMocks.saveSettings).toHaveBeenCalledOnce();
    expect(apiMocks.onError).toHaveBeenCalledWith(expect.stringContaining(backendError));
    expect(document.querySelector('[data-onboarding-step="3"]')).toBeTruthy();
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("First-run settings were not saved");
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBeNull();
    expect(apiMocks.addManagedScope).not.toHaveBeenCalled();
    expect(setView).not.toHaveBeenCalledWith("library");
  });

  it("allows a failed first save to be retried before completing Onboarding", async () => {
    const backendError = "settings_save_failure:watcher_runtime_failure";
    apiMocks.saveSettings
      .mockRejectedValueOnce(new Error(backendError))
      .mockImplementationOnce(async (request: SaveSettingsRequest) => ({
        settings: request.settings,
        revision: request.expectedRevision + 1
      }));
    renderOnboarding();
    await flushAsync();
    await selectFolderAtOnboardingStep();

    await clickNext();

    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBeNull();
    expect(apiMocks.addManagedScope).not.toHaveBeenCalled();
    expect(setView).not.toHaveBeenCalledWith("library");
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("First-run settings were not saved");

    await clickNext();

    expect(apiMocks.saveSettings).toHaveBeenCalledTimes(2);
    expect(document.querySelector('[data-onboarding-step="5"]')).toBeTruthy();
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBeNull();
    expect(apiMocks.addManagedScope).not.toHaveBeenCalled();

    await clickNext();
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(setView).toHaveBeenCalledWith("library");
  });

  it("fails closed when rollback returns an already equivalent enabled root", async () => {
    const backendError = "file watcher reload failed: injected watcher restart failure; settings were restored";
    const priorRoot = {
      ...createScanRootSetting("C:\\OwnerQualification\\fixture", "2026-10-08T00:00:00.000Z"),
      id: "backend-owned-root-id",
      path: "c:/ownerqualification/fixture/",
      label: "Fixture"
    };
    apiMocks.getSettings.mockResolvedValue({
      settings: { ...DEFAULT_APP_SETTINGS, defaultScanFolders: [priorRoot] },
      revision: 7
    });
    apiMocks.saveSettings.mockRejectedValue(new Error(backendError));
    renderOnboarding();
    await flushAsync();
    await selectFolderAtOnboardingStep();

    await clickNext();

    expect(apiMocks.getSettings).toHaveBeenCalledTimes(2);
    expect(apiMocks.saveSettings).toHaveBeenCalledOnce();
    expect(apiMocks.onError).toHaveBeenCalledWith(expect.stringContaining(backendError));
    expect(document.querySelector('[data-onboarding-step="3"]')).toBeTruthy();
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBeNull();
    expect(apiMocks.addManagedScope).not.toHaveBeenCalled();
    expect(setView).not.toHaveBeenCalledWith("library");
  });
});
