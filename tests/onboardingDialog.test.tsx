// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChromeProvider, type ChromeContextValue } from "../src/contexts/AppContexts";
import { makeTranslator } from "../src/i18n";
import { OnboardingDialog, ONBOARDING_STORAGE_KEY } from "../src/components/OnboardingDialog";
import { ONBOARDING_REOPEN_EVENT } from "../src/components/onboardingReopen";
import type { AIProductFeatureReadiness, GlobalIndexSource } from "../src/types/domain";

const apiMocks = vi.hoisted(() => ({
  getAIFeatureReadiness: vi.fn(),
  listGlobalIndexSources: vi.fn(),
  authorizeGlobalIndexSource: vi.fn(),
  setGlobalIndexSourceEnabled: vi.fn(),
  startGlobalIndex: vi.fn(),
  addManagedScope: vi.fn(),
  saveSettings: vi.fn(),
  getSettings: vi.fn(),
  startManagedScan: vi.fn()
}));

vi.mock("../src/api/tauriApi", () => ({ tauriApi: apiMocks }));

const t = makeTranslator("zh");

const readiness: AIProductFeatureReadiness = {
  provider: {
    state: "disabled",
    reason: "provider_disabled",
    providerMode: "cloud",
    providerKind: "openai_compatible",
    providerPreset: null,
    model: null,
    credentialRequired: true,
    credentialConfigured: false,
    settingsRevision: "fixture",
    bindingFingerprint: "fixture"
  },
  managedScopes: [],
  cleanup: {
    state: "disabled",
    reason: "provider_disabled",
    provider: {
      state: "disabled",
      reason: "provider_disabled",
      providerMode: "cloud",
      providerKind: "openai_compatible",
      providerPreset: null,
      model: null,
      credentialRequired: true,
      credentialConfigured: false,
      settingsRevision: "fixture",
      bindingFingerprint: "fixture"
    },
    cleanupAiEnabled: true,
    localAiAllowed: false,
    cloudAiAllowed: false,
    bindingFingerprint: "fixture",
    disclosure: {
      providerPayloadIncludesFileName: false,
      providerPayloadIncludesParentPath: false,
      providerPayloadIncludesFullPath: false,
      providerPayloadIncludesFileContent: false,
      providerContentIsBounded: true
    }
  }
};

function source(overrides: Partial<GlobalIndexSource["volume"]> = {}): GlobalIndexSource {
  return {
    volume: {
      id: "vol-c",
      platform: "windows",
      stableVolumeId: "stable-c",
      displayName: "Windows (C:)",
      mountPath: "C:\\",
      filesystemType: "NTFS",
      driveKind: "fixed",
      enabled: false,
      scopeAuthorizationState: "not_authorized",
      provider: "windows_mft_usn",
      indexStatus: "discovered",
      lastError: null,
      journalId: null,
      journalCursor: null,
      lastFullIndexAt: null,
      lastIncrementalSyncAt: null,
      entryCount: 0,
      createdAt: 0,
      updatedAt: 0,
      ...overrides
    },
    canPause: false,
    canRebuild: false,
    technicalDetail: null
  };
}

function makeChrome(setView = vi.fn()) {
  return { t, setView, view: "library", language: "zh", theme: "light", onError: vi.fn() } as unknown as ChromeContextValue;
}

function flushAsync() {
  return act(async () => {
    await Promise.resolve();
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    await Promise.resolve();
  });
}

describe("first-run product intro", () => {
  let root: Root;
  let setView: ReturnType<typeof vi.fn<(view: string) => void>>;
  const nativeGetClientRects = HTMLElement.prototype.getClientRects;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    localStorage.clear();
    sessionStorage.clear();
    document.body.innerHTML = '<div id="app-shell-content"><button id="background">Background</button></div><div id="test-root"></div>';
    HTMLElement.prototype.getClientRects = () => [{ width: 120, height: 40, top: 0, left: 0, right: 120, bottom: 40, x: 0, y: 0, toJSON() { return {}; } }] as unknown as DOMRectList;
    setView = vi.fn();
    apiMocks.getAIFeatureReadiness.mockReset().mockResolvedValue(readiness);
    apiMocks.listGlobalIndexSources.mockReset().mockResolvedValue([source()]);
    apiMocks.authorizeGlobalIndexSource.mockReset().mockResolvedValue(undefined);
    apiMocks.setGlobalIndexSourceEnabled.mockReset().mockResolvedValue(undefined);
    apiMocks.startGlobalIndex.mockReset().mockResolvedValue(undefined);
    apiMocks.addManagedScope.mockReset().mockResolvedValue({ id: "managed-scope" });
    apiMocks.saveSettings.mockReset().mockResolvedValue({ settings: {}, revision: 1 });
    apiMocks.getSettings.mockReset().mockResolvedValue({ settings: {}, revision: 0 });
    apiMocks.startManagedScan.mockReset().mockResolvedValue(undefined);
    root = createRoot(document.getElementById("test-root")!);
  });

  afterEach(() => {
    act(() => root.unmount());
    HTMLElement.prototype.getClientRects = nativeGetClientRects;
    document.body.innerHTML = "";
    localStorage.clear();
  });

  function renderIntro() {
    act(() => root.render(createElement(ChromeProvider, { value: makeChrome(setView), children: createElement(OnboardingDialog) })));
  }

  async function click(selector: string) {
    const button = document.querySelector<HTMLButtonElement>(selector);
    expect(button, selector).toBeTruthy();
    await act(async () => button?.click());
    await flushAsync();
  }

  function step() {
    return document.querySelector<HTMLElement>("[data-onboarding-step]")?.getAttribute("data-onboarding-step");
  }

  function expectNoIndexOrScopeSideEffects() {
    expect(apiMocks.addManagedScope).not.toHaveBeenCalled();
    expect(apiMocks.saveSettings).not.toHaveBeenCalled();
    expect(apiMocks.authorizeGlobalIndexSource).not.toHaveBeenCalled();
    expect(apiMocks.setGlobalIndexSourceEnabled).not.toHaveBeenCalled();
    expect(apiMocks.startGlobalIndex).not.toHaveBeenCalled();
    expect(apiMocks.startManagedScan).not.toHaveBeenCalled();
  }

  it("opens a four-scene intro on a clean profile and starts Files from page one", async () => {
    renderIntro();
    await flushAsync();

    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    expect(step()).toBe("1");
    expect(document.body.textContent).toContain("文件管理");
    // A representative Files composition is shown, not only bullet copy.
    expect(document.querySelector('[data-onboarding-showcase="files"]')).toBeTruthy();
    expect(document.body.textContent).toContain("不需要任何前置配置");

    await click("[data-onboarding-start]");

    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(setView).toHaveBeenCalledWith("library");
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expectNoIndexOrScopeSideEffects();
  });

  it("walks all four scenes and finishes into Files", async () => {
    renderIntro();
    await flushAsync();

    await click("[data-onboarding-next]");
    expect(step()).toBe("2");
    expect(document.body.textContent).toContain("AI 智能整理");
    expect(document.body.textContent).toContain("处理少量例外");

    await click("[data-onboarding-next]");
    expect(step()).toBe("3");
    expect(document.body.textContent).toContain("AI 垃圾清理");
    expect(document.body.textContent).toContain("Safe Trash");

    await click("[data-onboarding-next]");
    expect(step()).toBe("4");
    expect(document.body.textContent).toContain("全局搜索");
    expect(document.body.textContent).toContain("智能精简");
    // The volume list is a real read-only projection, not a promise.
    expect(apiMocks.listGlobalIndexSources).toHaveBeenCalled();
    expect(document.querySelector('[data-onboarding-search-source="vol-c"]')).toBeTruthy();

    await click("[data-onboarding-finish]");

    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(setView).toHaveBeenCalledWith("library");
    expectNoIndexOrScopeSideEffects();
  });

  it("lets every scene be skipped durably without configuring anything", async () => {
    renderIntro();
    await flushAsync();
    await click("[data-onboarding-skip]");

    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(setView).toHaveBeenCalledWith("library");
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expectNoIndexOrScopeSideEffects();

    // A restart keeps the intro closed: completion is durable.
    act(() => root.unmount());
    document.body.innerHTML = '<div id="app-shell-content"></div><div id="test-root"></div>';
    root = createRoot(document.getElementById("test-root")!);
    renderIntro();
    await flushAsync();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });

  it("treats Escape as a durable skip instead of a reopening loop", async () => {
    renderIntro();
    await flushAsync();
    expect(document.activeElement?.closest('[role="dialog"]')).toBeTruthy();

    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await flushAsync();

    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(setView).toHaveBeenCalledWith("library");
    expectNoIndexOrScopeSideEffects();
  });

  it("requires an explicit volume selection before enabling Global Search", async () => {
    renderIntro();
    await flushAsync();
    await click("[data-onboarding-next]");
    await click("[data-onboarding-next]");
    await click("[data-onboarding-next]");
    expect(step()).toBe("4");

    const enable = document.querySelector<HTMLButtonElement>("[data-onboarding-enable-search]");
    expect(enable?.disabled).toBe(true);

    const checkbox = document.querySelector<HTMLInputElement>('[data-onboarding-search-source="vol-c"]');
    expect(checkbox).toBeTruthy();
    await act(async () => checkbox?.click());
    expect(document.querySelector<HTMLButtonElement>("[data-onboarding-enable-search]")?.disabled).toBe(false);
    // Selecting a volume alone must not authorize or index it.
    expect(apiMocks.authorizeGlobalIndexSource).not.toHaveBeenCalled();
    expect(apiMocks.setGlobalIndexSourceEnabled).not.toHaveBeenCalled();

    await click("[data-onboarding-enable-search]");

    expect(apiMocks.authorizeGlobalIndexSource).toHaveBeenCalledWith("vol-c", true);
    expect(apiMocks.setGlobalIndexSourceEnabled).toHaveBeenCalledWith("vol-c", true);
    expect(apiMocks.startGlobalIndex).toHaveBeenCalledOnce();
    expect(document.querySelector("[data-onboarding-search-enabled]")).toBeTruthy();
    // Global Search consent never adds a Managed Library scope or a scan.
    expect(apiMocks.addManagedScope).not.toHaveBeenCalled();
    expect(apiMocks.saveSettings).not.toHaveBeenCalled();
    expect(apiMocks.startManagedScan).not.toHaveBeenCalled();
  });

  it("skips Global Search without authorizing any volume", async () => {
    renderIntro();
    await flushAsync();
    await click("[data-onboarding-next]");
    await click("[data-onboarding-next]");
    await click("[data-onboarding-next]");

    await click("[data-onboarding-finish]");

    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(setView).toHaveBeenCalledWith("library");
    expectNoIndexOrScopeSideEffects();
  });

  it("keeps the volume step usable when the source projection fails", async () => {
    apiMocks.listGlobalIndexSources.mockRejectedValue(new Error("global_index_unavailable"));
    renderIntro();
    await flushAsync();
    await click("[data-onboarding-next]");
    await click("[data-onboarding-next]");
    await click("[data-onboarding-next]");

    expect(document.querySelector('[data-onboarding-search-state="failed"]')).toBeTruthy();
    expect(document.body.textContent).toContain("无法读取可用磁盘");
    expect(document.querySelector<HTMLButtonElement>("[data-onboarding-enable-search]")?.disabled).toBe(true);

    await click("[data-onboarding-finish]");
    expect(setView).toHaveBeenCalledWith("library");
    expectNoIndexOrScopeSideEffects();
  });

  it("routes the AI configuration entry to existing AI Settings and completes the intro", async () => {
    renderIntro();
    await flushAsync();
    await click("[data-onboarding-next]");

    await click("[data-onboarding-configure-ai]");

    expect(setView).toHaveBeenCalledWith("settings");
    expect(localStorage.getItem(ONBOARDING_STORAGE_KEY)).toBe("true");
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expectNoIndexOrScopeSideEffects();
  });

  it("reopens the intro from the voluntary About entry without leaving the current page", async () => {
    localStorage.setItem(ONBOARDING_STORAGE_KEY, "true");
    renderIntro();
    await flushAsync();
    expect(document.querySelector('[role="dialog"]')).toBeNull();

    await act(async () => window.dispatchEvent(new CustomEvent(ONBOARDING_REOPEN_EVENT)));
    await flushAsync();

    expect(document.querySelector('[role="dialog"]')).toBeTruthy();
    expect(step()).toBe("1");

    await click("[data-onboarding-skip]");
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    // A voluntary re-read never bounces the user away from Settings.
    expect(setView).not.toHaveBeenCalled();
    expectNoIndexOrScopeSideEffects();
  });

  it("keeps the dialog scrollable with reachable controls at narrow sizes", async () => {
    renderIntro();
    await flushAsync();
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
    const description = document.querySelector<HTMLElement>("#onboarding-description");
    const footer = dialog?.querySelector("footer");
    expect(dialog?.className).toContain("w-full max-w-4xl");
    expect(dialog?.className).toContain("max-h-[calc(100dvh-1.5rem)]");
    expect(description?.className).toContain("overflow-y-auto");
    expect(description?.className).toContain("overscroll-contain");
    expect(footer?.className).toContain("flex-wrap");
    expect(document.querySelector<HTMLButtonElement>("[data-onboarding-start]")?.disabled).toBe(false);
  });
});
