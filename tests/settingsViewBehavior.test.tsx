// @vitest-environment happy-dom

import { act } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { GlobalSearchSettingsSection, type GlobalSearchSettingsSectionProps } from "../src/views/settings/sections/GlobalSearchSettingsSection";
import { makeTranslator } from "../src/i18n";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AIProviderPreset, AISettings, FileLibraryDetail } from "../src/types/domain";
import { requestSettingsSection } from "../src/components/spotlight/commandRegistry";
import { AI_SETTINGS_MODE_REQUEST_KEY } from "../src/views/settings/settingsNavigation";
import { useFileLibraryInspectorStore, useFileLibrarySelectionStore } from "../src/store/useFileLibraryV2Store";

const mocks = vi.hoisted(() => ({
  getAISettings: vi.fn(),
  getRuntimeCapabilities: vi.fn(),
  listAIProviderPresets: vi.fn(),
  saveAISettings: vi.fn(),
  publishAIProcessingMode: vi.fn(),
  getGlobalHotkeyStatus: vi.fn(),
  testAIProviderConnection: vi.fn(),
  debugAIClassificationOnce: vi.fn(),
  getFileLibraryDetail: vi.fn(),
  updateSettings: vi.fn(),
  scanPath: vi.fn(),
  enqueueRoot: vi.fn(),
  setGlobalHotkeyError: vi.fn(),
  getGlobalIndexStatus: vi.fn(),
  listGlobalIndexSources: vi.fn(),
  listManagedScopes: vi.fn(),
  getAiManagementStatus: vi.fn(),
  runtimeState: {
    status: "ready" as const,
    settings: { enabled: true, provider: "openai_compatible" as const }
  }
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

vi.mock("../src/api/tauriApi", () => ({
  tauriApi: {
    getAISettings: mocks.getAISettings,
    getRuntimeCapabilities: mocks.getRuntimeCapabilities,
    listAIProviderPresets: mocks.listAIProviderPresets,
    saveAISettings: mocks.saveAISettings,
    getGlobalHotkeyStatus: mocks.getGlobalHotkeyStatus,
    getGlobalIndexStatus: mocks.getGlobalIndexStatus,
    listGlobalIndexSources: mocks.listGlobalIndexSources,
    listManagedScopes: mocks.listManagedScopes,
    getAiManagementStatus: mocks.getAiManagementStatus,
    testAIProviderConnection: mocks.testAIProviderConnection,
    debugAIClassificationOnce: mocks.debugAIClassificationOnce,
    getFileLibraryDetail: mocks.getFileLibraryDetail
  }
}));

vi.mock("../src/contexts/AppContexts", async () => {
  const { makeTranslator } = await import("../src/i18n");
  const saved = vi.fn(async () => true);
  return {
    useI18nContext: () => ({
      language: "en",
      setLanguage: vi.fn(),
      t: makeTranslator("en")
    }),
    useNavigationContext: () => ({
      theme: "light",
      setView: vi.fn(),
      onError: vi.fn()
    }),
    useCommandContext: () => ({
      platform: "browser",
      commandInputRef: { current: null },
      isCommandOpen: false,
      setIsCommandOpen: vi.fn(),
      hotkeyLabel: "Ctrl+K",
      isSearchMode: false,
    }),
    useThemeContext: () => ({
      theme: "light",
      setTheme: vi.fn(),
      effectiveTheme: "light"
    }),
    useWindowContext: () => ({
      closeBehavior: "ask",
      setCloseBehavior: saved,
      isWindows: false,
      isCloseChoiceOpen: false,
      onCancelCloseChoice: vi.fn(),
      handleWindowAction: vi.fn(),
      requestClose: vi.fn(),
      resolveCloseChoice: vi.fn()
    }),
    useSettingsContext: () => ({
      settings: {
        folderNamingLanguage: "en",
        defaultScanFolders: [],
        restoreRetentionDays: 30,
        launchAtLogin: false,
        backgroundIndexOnStartup: false,
        searchHotkey: "CmdOrCtrl+K",
        searchScopeMode: "all",
        customSearchRoots: [],
        organizeRootMode: "current_folder",
        organizeRootPath: undefined,
        useLegacyBuiltinClassificationRules: false,
        useLearnedRulesAsAutoRules: false
      },
      updateSettings: mocks.updateSettings,
      setFolderNamingLanguage: saved,
      setDefaultScanFolders: saved,
      setRestoreRetentionDays: saved,
      setLaunchAtLogin: saved,
      setBackgroundIndexOnStartup: saved,
      setSearchHotkey: saved,
      setSearchScopeMode: saved,
      setCustomSearchRoots: saved,
      setOrganizeRootMode: saved,
      setOrganizeRootPath: saved
    })
  };
});

vi.mock("../src/store/useScanManagerStore", () => ({
  useScanManagerStore: (selector: (state: { scanPath: typeof mocks.scanPath }) => unknown) => selector({ scanPath: mocks.scanPath })
}));

vi.mock("../src/store/useBackgroundIndexerStore", () => ({
  useBackgroundIndexerStore: (selector: (state: {
    pendingRoots: string[];
    currentRoot: string | null;
    isBackgroundIndexing: boolean;
    failedRoots: Array<{ path: string; message: string }>;
    completedRoots: string[];
    enqueueRoot: typeof mocks.enqueueRoot;
  }) => unknown) => selector({
    pendingRoots: [],
    currentRoot: null,
    isBackgroundIndexing: false,
    failedRoots: [],
    completedRoots: [],
    enqueueRoot: mocks.enqueueRoot
  })
}));

vi.mock("../src/store/useAppStore", () => ({
  useAppStore: (selector: (state: { globalHotkeyError: string; setGlobalHotkeyError: typeof mocks.setGlobalHotkeyError }) => unknown) => selector({
    globalHotkeyError: "",
    setGlobalHotkeyError: mocks.setGlobalHotkeyError
  })
}));

vi.mock("../src/store/useAIProcessingModeStore", () => ({
  useAIProcessingModeStore: (selector: (state: { status: "ready"; settings: { enabled: boolean; provider: "openai_compatible" }; publish: typeof mocks.publishAIProcessingMode }) => unknown) => selector({
    status: mocks.runtimeState.status,
    settings: mocks.runtimeState.settings,
    publish: mocks.publishAIProcessingMode
  })
}));

import { SettingsView } from "../src/views/settings/SettingsView";

const secret = ["dynamic", "qa", "sentinel", Math.random().toString(36).slice(2)].join("-");

const cloudPreset: AIProviderPreset = {
  id: "deepseek",
  label: "DeepSeek",
  providerKind: "openai_compatible",
  defaultBaseUrl: "https://api.deepseek.com",
  defaultChatPath: "/chat/completions",
  defaultModel: "deepseek-chat",
  supportsResponseFormat: true,
  supportsThinking: true,
  supportsReasoningEffort: false
};

const customPreset: AIProviderPreset = {
  ...cloudPreset,
  id: "custom_openai_compatible",
  label: "Custom",
  defaultBaseUrl: "https://custom.example",
  defaultModel: "custom-model"
};

const localPreset: AIProviderPreset = {
  ...cloudPreset,
  id: "ollama",
  label: "Ollama",
  providerKind: "ollama",
  defaultBaseUrl: "http://127.0.0.1:11434",
  defaultChatPath: "/api/chat",
  defaultModel: "qwen3",
  supportsResponseFormat: false,
  supportsThinking: false
};

function initialAISettings(): AISettings {
  return {
    enabled: true,
    provider: "openai_compatible",
    preset: "deepseek",
    baseUrl: "https://api.deepseek.com/custom",
    chatPath: "/chat/completions",
    apiKey: "",
    apiKeyAction: "preserve",
    apiKeyConfigured: true,
    model: "preserved-model",
    temperature: 0,
    maxTokens: 1024,
    batchSize: 10,
    classificationConcurrency: 2,
    timeoutSeconds: 120,
    sendFullPath: false,
    sendParentPath: true,
    classificationMode: "ai_first",
    cleanupAiEnabled: true,
    cleanupLocalAiAllowed: false,
    cleanupCloudAiAllowed: false,
    forceJsonOutput: false,
    enableThinking: false,
    reasoningEffort: null,
    extraBodyJson: null
  };
}

function libraryDetail(id: string): FileLibraryDetail {
  return { id, name: id + ".txt", path: "D:/Library/" + id + ".txt" } as FileLibraryDetail;
}

let container: HTMLDivElement;
let root: Root;

async function flushEffects() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function changeInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, value);
  await act(async () => input.dispatchEvent(new Event("input", { bubbles: true })));
}

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  window.localStorage.setItem("zc-developer-mode", "true");
  window.sessionStorage.clear();
  if (!HTMLElement.prototype.scrollIntoView) HTMLElement.prototype.scrollIntoView = () => undefined;
  vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => undefined);
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    callback(0);
    return 1;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
  mocks.getAISettings.mockResolvedValue(initialAISettings());
  mocks.getRuntimeCapabilities.mockResolvedValue({
    aiDebugAvailable: true,
    realAIClassificationAvailable: true,
    credentialStoreAvailable: true
  });
  mocks.listAIProviderPresets.mockResolvedValue([cloudPreset, customPreset, localPreset]);
  mocks.getGlobalHotkeyStatus.mockResolvedValue({ error: null });
  mocks.getGlobalIndexStatus.mockResolvedValue({ platform: "browser", enabled: true, status: "ready", processedEntries: 0, collectionComplete: true, totalEntries: 0, indexedVolumes: 0, readyVolumes: 0, pendingVolumes: 0, lastSyncAt: null, lastError: null });
  mocks.listGlobalIndexSources.mockResolvedValue([]);
  mocks.listManagedScopes.mockResolvedValue([]);
  mocks.getAiManagementStatus.mockResolvedValue({ enabledScopeCount: 0, managedEntryCount: 0, pendingJobCount: 0, runningJobCount: 0, cloudScopeCount: 0, policySummary: "managed_scope_only_cloud_disabled" });
  mocks.getFileLibraryDetail.mockReset();
  useFileLibrarySelectionStore.getState().clear();
  useFileLibraryInspectorStore.getState().clear();
  mocks.updateSettings.mockResolvedValue(true);
  mocks.runtimeState.settings = { enabled: true, provider: "openai_compatible" };
  mocks.publishAIProcessingMode.mockImplementation((settings: { enabled: boolean; provider: "openai_compatible" }) => {
    mocks.runtimeState.settings = settings;
  });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(<SettingsView />));
  await flushEffects();
  mocks.publishAIProcessingMode.mockClear();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  window.localStorage.clear();
  window.sessionStorage.clear();
  vi.clearAllMocks();
  vi.restoreAllMocks();
});

describe("settings view behavior", () => {
  it.each([
    ["local", "Using a local model"],
    ["cloud", "Using cloud AI"]
  ] as const)("consumes onboarding's %s mode request in the existing Settings controls", async (mode, label) => {
    await act(async () => root.render(null));
    window.sessionStorage.setItem(AI_SETTINGS_MODE_REQUEST_KEY, mode);
    await act(async () => root.render(<SettingsView />));
    await flushEffects();

    const selected = [...container.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="AI mode"] [role="radio"]')]
      .find((radio) => radio.textContent === label);
    expect(selected?.getAttribute("aria-checked")).toBe("true");
    expect(window.sessionStorage.getItem(AI_SETTINGS_MODE_REQUEST_KEY)).toBeNull();
  });

  it("uses the explicit-single V2 selection and matching Inspector detail for the AI debug target", async () => {
    mocks.getFileLibraryDetail.mockResolvedValue(libraryDetail("settings-file"));
    await act(async () => useFileLibrarySelectionStore.getState().setExplicit(["settings-file"], "settings-file", -1));
    await flushEffects();

    expect(mocks.getFileLibraryDetail).toHaveBeenCalledTimes(1);
    expect(mocks.getFileLibraryDetail).toHaveBeenCalledWith("settings-file");

    const debugSummary = [...container.querySelectorAll<HTMLElement>("summary")]
      .find((summary) => summary.textContent?.includes("Debug one file's AI response"));
    await act(async () => debugSummary?.click());
    expect(container.textContent).toContain("settings-file.txt");

    const useSelectedFile = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Use selected file");
    await act(async () => useSelectedFile?.click());
    expect(container.querySelector<HTMLInputElement>("#settings-ai-debug-target")?.value).toBe("settings-file");
  });

  it("keeps multi-selection and Inspector failures fail-closed without legacy fallback or retry", async () => {
    await act(async () => useFileLibraryInspectorStore.setState({
      selectedId: "legacy-file",
      detail: libraryDetail("legacy-file"),
      isLoading: false,
      error: null
    }));
    await act(async () => useFileLibrarySelectionStore.getState().setExplicit(["settings-a", "settings-b"], "settings-a", 0));
    await flushEffects();

    expect(mocks.getFileLibraryDetail).not.toHaveBeenCalled();
    const debugSummary = [...container.querySelectorAll<HTMLElement>("summary")]
      .find((summary) => summary.textContent?.includes("Debug one file's AI response"));
    await act(async () => debugSummary?.click());
    expect(container.textContent).not.toContain("legacy-file.txt");

    mocks.getFileLibraryDetail.mockRejectedValue(new Error("detail_failed"));
    await act(async () => useFileLibrarySelectionStore.getState().setExplicit(["failed-file"], "failed-file", -1));
    await flushEffects();
    await flushEffects();
    expect(mocks.getFileLibraryDetail).toHaveBeenCalledTimes(1);
    expect(container.textContent).not.toContain("failed-file.txt");
  });

  it("hides AI debug when the runtime denies it even if developer mode is enabled", async () => {
    await act(async () => root.render(null));
    mocks.getRuntimeCapabilities.mockResolvedValue({
      aiDebugAvailable: false,
      realAIClassificationAvailable: true,
      credentialStoreAvailable: true
    });
    await act(async () => root.render(<SettingsView />));
    await flushEffects();

    expect(container.textContent).not.toContain("Debug one file's AI response");
    expect(container.textContent).toContain("Test connection");
    expect(container.querySelector("#settings-ai-api-key")).not.toBeNull();
  });

  it("keeps credential and connection controls available when developer mode is disabled", async () => {
    await act(async () => root.render(null));
    window.localStorage.setItem("zc-developer-mode", "false");
    mocks.getRuntimeCapabilities.mockResolvedValue({
      aiDebugAvailable: false,
      realAIClassificationAvailable: true,
      credentialStoreAvailable: true
    });
    await act(async () => root.render(<SettingsView />));
    await flushEffects();

    expect(container.textContent).not.toContain("Debug one file's AI response");
    expect(container.textContent).toContain("Advanced settings");
    expect(container.textContent).toContain("Test connection");
    expect(container.querySelector("#settings-ai-api-key")).not.toBeNull();
    expect(container.querySelector("#settings-ai-base-url")).not.toBeNull();
    expect(container.querySelector("#settings-ai-model")).not.toBeNull();
    expect(container.querySelector("#settings-ai-batch-size")).toBeNull();
  });

  it("keeps runtime Off while a Cloud draft is unsaved, then publishes Cloud only after success", async () => {
    await act(async () => root.render(null));
    mocks.runtimeState.settings = { enabled: false, provider: "openai_compatible" };
    mocks.getAISettings.mockResolvedValue({ ...initialAISettings(), enabled: false });
    await act(async () => root.render(<SettingsView />));
    await flushEffects();
    mocks.publishAIProcessingMode.mockClear();

    const cloud = [...container.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="AI mode"] [role="radio"]')]
      .find((radio) => radio.textContent === "Using cloud AI")!;
    await act(async () => cloud.click());
    expect(container.querySelector("[data-ai-save-bar]")?.textContent).toContain("Current active AI mode: AI is off");
    expect(container.querySelector("[data-ai-save-bar]")?.textContent).toContain("Unsaved draft mode: Using cloud AI");
    expect(mocks.publishAIProcessingMode).not.toHaveBeenCalled();

    mocks.saveAISettings.mockImplementation(async (next: AISettings) => next);
    mocks.getAISettings.mockResolvedValue({ ...initialAISettings(), enabled: true });
    const save = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save AI settings")!;
    await act(async () => save.click());
    await flushEffects();
    expect(mocks.publishAIProcessingMode).toHaveBeenCalledWith({ enabled: true, provider: "openai_compatible" });
    expect(container.querySelector("[data-ai-save-bar]")?.textContent).toContain("Current active AI mode: Using cloud AI");
    expect(container.querySelector("[data-ai-unsaved-draft]")).toBeNull();
  });

  it("starts every normal entry at General with a zeroed Settings scroll owner", async () => {
    const oldScrollOwner = container.querySelector<HTMLElement>("[data-settings-scroll-container]")!;
    oldScrollOwner.scrollTop = 640;
    await act(async () => root.render(null));
    await act(async () => root.render(<SettingsView />));
    await flushEffects();
    const scrollOwner = container.querySelector<HTMLElement>("[data-settings-scroll-container]")!;
    expect(scrollOwner.scrollTop).toBe(0);
    expect(container.querySelector('[data-settings-section="settings-general"]')?.getAttribute("aria-current")).toBe("location");
    expect(container.querySelectorAll("[data-settings-scroll-container]")).toHaveLength(1);
  });

  it("keeps direct and pending Spotlight section requests active and focuses the requested heading", async () => {
    await act(async () => requestSettingsSection("settings-ai"));
    expect(container.querySelector('[data-settings-section="settings-ai"]')?.getAttribute("aria-current")).toBe("location");
    expect(document.activeElement).toBe(container.querySelector("#settings-ai-heading"));

    await act(async () => root.render(null));
    requestSettingsSection("settings-privacy");
    await act(async () => root.render(<SettingsView />));
    await flushEffects();
    expect(container.querySelector('[data-settings-section="settings-privacy"]')?.getAttribute("aria-current")).toBe("location");
    expect(document.activeElement).toBe(container.querySelector("#settings-privacy-heading"));
  });

  it("localizes global index privacy errors instead of exposing provider codes", async () => {
    await act(async () => root.render(null));
    mocks.getGlobalIndexStatus.mockResolvedValue({
      platform: "darwin",
      enabled: true,
      status: "permission_required",
      processedEntries: 24,
      collectionComplete: true,
      totalEntries: 24,
      indexedVolumes: 1,
      readyVolumes: 0,
      pendingVolumes: 0,
      lastSyncAt: null,
      lastError: "macos_spotlight_full_disk_access_required"
    });
    await act(async () => root.render(<SettingsView />));
    await flushEffects();

    expect(container.textContent).toContain("Full Disk Access");
    expect(container.textContent).not.toContain("macos_spotlight_full_disk_access_required");
    expect(container.textContent).toContain("Processed");
    expect(container.textContent).toContain("Initial collection complete");
  });

  it("disables AI-dependent controls while off and restores preserved configuration when re-enabled", async () => {
    const modeGroup = container.querySelector<HTMLElement>('[role="radiogroup"][aria-label="AI mode"]')!;
    const modeRadios = [...modeGroup.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
    const off = modeRadios.find((radio) => radio.textContent === "AI is off")!;
    const cloud = modeRadios.find((radio) => radio.textContent === "Using cloud AI")!;
    const apiKey = container.querySelector<HTMLInputElement>("#settings-ai-api-key")!;
    const baseUrl = container.querySelector<HTMLInputElement>("#settings-ai-base-url")!;
    const model = container.querySelector<HTMLInputElement>("#settings-ai-model")!;

    await act(async () => off.click());

    expect(modeRadios.every((radio) => !radio.disabled)).toBe(true);
    expect(container.querySelector<HTMLInputElement>("#settings-ai-cleanup")?.disabled).toBe(true);
    expect(container.querySelector<HTMLSelectElement>("#settings-ai-provider")?.disabled).toBe(true);
    expect(container.querySelector<HTMLElement>('[role="radiogroup"][aria-label="AI classification presets"]')?.getAttribute("aria-disabled")).toBe("true");
    expect(baseUrl.disabled).toBe(true);
    expect(apiKey.disabled).toBe(true);
    expect(model.disabled).toBe(true);
    expect(baseUrl.value).toBe("https://api.deepseek.com/custom");
    expect(apiKey.value).toBe("");
    expect(model.value).toBe("preserved-model");
    expect(container.textContent).toContain("Existing provider and request settings are preserved");
    expect(container.querySelector<HTMLButtonElement>("button[type='button']:not([data-settings-secret-toggle])")?.disabled).toBe(false);

    const save = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save AI settings")!;
    expect(save.disabled).toBe(false);

    await act(async () => cloud.click());
    expect(container.querySelector<HTMLInputElement>("#settings-ai-cleanup")?.disabled).toBe(false);
    expect(baseUrl.value).toBe("https://api.deepseek.com/custom");
    expect(apiKey.value).toBe("");
    expect(model.value).toBe("preserved-model");
  });

  it("keeps the API key when the provider changes", async () => {
    const provider = container.querySelector<HTMLSelectElement>("#settings-ai-provider")!;
    const apiKey = container.querySelector<HTMLInputElement>("#settings-ai-api-key")!;
    expect(apiKey.value).toBe("");
    provider.value = "custom";
    await act(async () => provider.dispatchEvent(new Event("change", { bubbles: true })));
    expect(container.querySelector<HTMLInputElement>("#settings-ai-api-key")?.value).toBe("");
    expect([...container.querySelectorAll("button")].some((button) => button.textContent === "Clear saved API key")).toBe(true);
  });

  it("uses explicit key actions, clears the visible secret, and rejects failed read-back", async () => {
    const apiKey = container.querySelector<HTMLInputElement>("#settings-ai-api-key")!;
    await changeInput(apiKey, secret);
    mocks.saveAISettings.mockImplementation(async (next: AISettings) => ({ ...next, apiKey: "" }));
    mocks.getAISettings.mockResolvedValue({ ...initialAISettings(), apiKeyConfigured: false });

    const save = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save AI settings")!;
    await act(async () => save.click());
    await flushEffects();

    expect(mocks.saveAISettings).toHaveBeenCalledWith(expect.objectContaining({ apiKey: secret, apiKeyAction: "replace" }));
    expect(container.querySelector<HTMLInputElement>("#settings-ai-api-key")?.value).toBe("");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Credential read-back verification failed after saving");
    expect(mocks.publishAIProcessingMode).not.toHaveBeenCalled();
  });

  it("retains a failed AI draft, keeps runtime unchanged, and exposes one redacted retryable alert", async () => {
    mocks.saveAISettings.mockRejectedValue(new Error(`backend rejected ${secret}`));
    const apiKey = container.querySelector<HTMLInputElement>("#settings-ai-api-key")!;
    await changeInput(apiKey, secret);
    const modeGroup = container.querySelector<HTMLElement>('[role="radiogroup"][aria-label="AI mode"]')!;
    const off = [...modeGroup.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find((radio) => radio.textContent === "AI is off")!;
    await act(async () => off.click());
    const save = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save AI settings")!;
    await act(async () => save.click());
    await flushEffects();

    expect(mocks.saveAISettings).toHaveBeenCalledTimes(1);
    expect(mocks.publishAIProcessingMode).not.toHaveBeenCalled();
    const alerts = [...container.querySelectorAll<HTMLElement>('[role="alert"]')];
    expect(alerts).toHaveLength(1);
    expect(alerts[0].textContent).toContain("Failed to save AI settings");
    expect(alerts[0].textContent).toContain("[redacted]");
    expect(container.textContent).not.toContain(secret);
    expect(modeGroup.querySelector('[role="radio"][aria-checked="true"]')?.textContent).toBe("AI is off");
    expect(container.querySelector("[data-ai-save-bar]")?.textContent).toContain("Current active AI mode: Using cloud AI");
    expect(container.querySelector("[data-ai-save-bar]")?.textContent).toContain("Unsaved draft mode: AI is off");
    expect(container.querySelector('[data-ai-settings-state="error"]')?.textContent).toBe("The last successfully saved AI settings remain active.");
    expect(save.disabled).toBe(false);

    mocks.saveAISettings.mockResolvedValue({ ...initialAISettings(), enabled: false });
    mocks.getAISettings.mockResolvedValue({ ...initialAISettings(), enabled: false });
    await act(async () => save.click());
    await flushEffects();
    expect(mocks.publishAIProcessingMode).toHaveBeenCalledWith({ enabled: false, provider: "openai_compatible" });
    expect(container.querySelectorAll('[role="alert"]')).toHaveLength(0);
    expect(container.querySelector("fieldset")?.getAttribute("data-ai-dirty")).toBe("false");
  });

  it("publishes runtime only after save success and clears the dirty draft", async () => {
    const saved = { ...initialAISettings(), enabled: false };
    mocks.saveAISettings.mockResolvedValue(saved);
    mocks.getAISettings.mockResolvedValue(saved);
    const modeGroup = container.querySelector<HTMLElement>('[role="radiogroup"][aria-label="AI mode"]')!;
    const off = [...modeGroup.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find((radio) => radio.textContent === "AI is off")!;
    await act(async () => off.click());
    const save = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save AI settings")!;
    await act(async () => save.click());
    await flushEffects();

    expect(mocks.publishAIProcessingMode).toHaveBeenCalledWith({ enabled: false, provider: "openai_compatible" });
    expect(container.querySelector("fieldset")?.getAttribute("data-ai-dirty")).toBe("false");
    expect(container.querySelector("[data-ai-unsaved-draft]")).toBeNull();
    expect(save.disabled).toBe(true);
    expect(container.querySelectorAll('[role="alert"]')).toHaveLength(0);
    expect(container.querySelector('[data-ai-status-region] [role="status"]')?.textContent).toContain("AI settings saved");
    expect(container.querySelector("[data-ai-status-region]")?.querySelectorAll('[role="status"]')).toHaveLength(1);
  });

  it("has one editable provider and hides a revealed API key on provider, section, mode, advanced, and save transitions", async () => {
    expect(container.querySelectorAll("select[id*='provider']")).toHaveLength(1);
    const details = container.querySelector<HTMLDetailsElement>("#settings-ai details")!;
    await act(async () => details.querySelector("summary")?.click());
    const reveal = () => container.querySelector<HTMLButtonElement>("[data-settings-secret-toggle]")?.click();
    const keyType = () => container.querySelector<HTMLInputElement>("#settings-ai-api-key")?.type;

    await act(async () => reveal());
    expect(keyType()).toBe("text");
    const provider = container.querySelector<HTMLSelectElement>("#settings-ai-provider")!;
    provider.value = "custom";
    await act(async () => provider.dispatchEvent(new Event("change", { bubbles: true })));
    expect(keyType()).toBe("password");

    await act(async () => reveal());
    await act(async () => container.querySelector<HTMLButtonElement>('[data-settings-section="settings-privacy"]')?.click());
    expect(keyType()).toBe("password");
    await act(async () => container.querySelector<HTMLButtonElement>('[data-settings-section="settings-ai"]')?.click());

    await act(async () => reveal());
    await act(async () => container.querySelector<HTMLButtonElement>('[data-settings-section="settings-privacy"]')?.click());
    expect(keyType()).toBe("password");
    await act(async () => container.querySelector<HTMLButtonElement>('[data-settings-section="settings-ai"]')?.click());

    await act(async () => reveal());
    const off = [...container.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="AI mode"] [role="radio"]')].find((radio) => radio.textContent === "AI is off")!;
    await act(async () => off.click());
    expect(keyType()).toBe("password");

    const cloud = [...container.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="AI mode"] [role="radio"]')].find((radio) => radio.textContent === "Using cloud AI")!;
    await act(async () => cloud.click());
    await act(async () => reveal());
    await act(async () => details.querySelector("summary")?.click());
    expect(keyType()).toBe("password");

    await act(async () => details.querySelector("summary")?.click());
    await act(async () => reveal());
    const model = container.querySelector<HTMLInputElement>("#settings-ai-model")!;
    model.value = "saved-model";
    await act(async () => model.dispatchEvent(new Event("input", { bubbles: true })));
    mocks.saveAISettings.mockImplementation(async (next: AISettings) => next);
    mocks.getAISettings.mockResolvedValue({ ...initialAISettings(), model: "saved-model" });
    const save = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save AI settings")!;
    await act(async () => save.click());
    await flushEffects();
    expect(keyType()).toBe("password");

    await act(async () => reveal());
    const fastPreset = [...container.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="AI classification presets"] [role="radio"]')]
      .find((radio) => radio.textContent === "Fast")!;
    await act(async () => fastPreset.click());
    expect(keyType()).toBe("text");
    mocks.saveAISettings.mockRejectedValueOnce(new Error("expected test failure"));
    await act(async () => save.click());
    await flushEffects();
    expect(keyType()).toBe("password");

    await act(async () => container.querySelector<HTMLButtonElement>('[data-settings-section="settings-about"]')?.click());
    const developerSwitch = container.querySelector<HTMLInputElement>("#settings-developer-mode")!;
    await act(async () => developerSwitch.click());
    expect(container.querySelector<HTMLInputElement>("#settings-ai-api-key")?.type).toBe("password");
    expect(container.querySelector("#settings-ai-batch-size")).toBeNull();

    await act(async () => root.render(null));
    window.localStorage.setItem("zc-developer-mode", "true");
    await act(async () => root.render(<SettingsView />));
    await flushEffects();
    expect(container.querySelector<HTMLInputElement>("#settings-ai-api-key")?.type).toBe("password");
  });
});


it("does not offer Windows folder-index controls as Global Search coverage", () => {
  const noop = vi.fn();
  const t = makeTranslator("en");
  const props: GlobalSearchSettingsSectionProps = {
    t, platform: "win32", searchHotkey: "CmdOrCtrl+K", hotkey: "Ctrl K", isRecordingHotkey: false,
    recordingHotkeyPreview: "", hotkeyCaptureRef: { current: null }, globalHotkeyError: "", globalHotkeyStatus: null,
    searchScopeMode: "custom_roots", customSearchRoots: [{ id: "unindexed-f", path: "F:/owned-fixture", label: "F fixture", enabled: true, createdAt: "2026-10-24" }],
    pendingBackgroundRoots: [], currentBackgroundRoot: null, isBackgroundIndexing: false, completedBackgroundRoots: ["F:/owned-fixture"], failedBackgroundRoots: [],
    onStartRecording: noop, onUpdateHotkey: noop, onSearchScopeMode: noop, onAddSearchRoot: noop, onSetSearchRootEnabled: noop,
    onIndexSearchRootNow: noop, onRequestDelete: noop, backgroundRootState: () => "completed"
  };
  const markup = renderToStaticMarkup(<GlobalSearchSettingsSection {...props} />);
  expect(markup).toContain(t("globalSearchWindowsScopeDesc"));
  expect(markup).toContain(t("globalSearchManage"));
  expect(markup).not.toContain(t("indexNow"));
  expect(markup).not.toContain(t("searchScopeCustomRoots"));
  expect(noop).not.toHaveBeenCalled();
});
