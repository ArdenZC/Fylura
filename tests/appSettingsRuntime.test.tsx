// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createScanRootSetting, DEFAULT_APP_SETTINGS, useAppSettings } from "../src/hooks/useAppSettings";

const settingsMocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  saveSettings: vi.fn(),
  onError: vi.fn()
}));

vi.mock("../src/api/tauriApi", () => ({
  tauriApi: {
    getSettings: settingsMocks.getSettings,
    saveSettings: settingsMocks.saveSettings
  }
}));

describe("useAppSettings load and save epochs", () => {
  let root: ReturnType<typeof createRoot> | undefined;
  let latest: ReturnType<typeof useAppSettings> | undefined;

  beforeEach(() => {
    settingsMocks.getSettings.mockReset();
    settingsMocks.saveSettings.mockReset();
    settingsMocks.onError.mockReset();
    latest = undefined;
  });

  afterEach(() => {
    root?.unmount();
    root = undefined;
  });

  it("does not let a delayed initial load overwrite a user intent", async () => {
    const loaded = deferred<ReturnType<typeof versionedSettings>>();
    settingsMocks.getSettings.mockReturnValue(loaded.promise);
    const container = document.createElement("div");
    root = createRoot(container);
    root.render(createElement(SettingsHarness, { onState: (state) => { latest = state; } }));
    await flushPromises();

    expect(latest?.isLoadingSettings).toBe(true);
    const saving = latest?.updateSettings({ searchHotkey: "Ctrl+Shift+P" });
    expect(settingsMocks.saveSettings).not.toHaveBeenCalled();

    loaded.resolve({
      settings: { ...DEFAULT_APP_SETTINGS, searchHotkey: "Alt+K" },
      revision: 7
    });
    settingsMocks.saveSettings.mockResolvedValue({
      settings: { ...DEFAULT_APP_SETTINGS, searchHotkey: "Ctrl+Shift+P" },
      revision: 8
    });
    await saving;
    await flushPromises();

    expect(settingsMocks.saveSettings).toHaveBeenCalledWith({
      settings: { ...DEFAULT_APP_SETTINGS, searchHotkey: "Ctrl+Shift+P" },
      expectedRevision: 7
    });
    expect(latest?.settings.searchHotkey).toBe("Ctrl+Shift+P");
  });

  it("preserves the first-run scan scope across delayed load and a CAS conflict", async () => {
    const loaded = deferred<ReturnType<typeof versionedSettings>>();
    const selectedRoot = createScanRootSetting(
      "C:/OwnerQualification/fixture",
      "2026-10-08T00:00:00.000Z"
    );
    const newerSettings = {
      ...DEFAULT_APP_SETTINGS,
      backgroundIndexOnStartup: false
    };
    settingsMocks.getSettings.mockReturnValue(loaded.promise);
    settingsMocks.saveSettings
      .mockRejectedValueOnce(new Error("settings_revision_conflict"))
      .mockResolvedValueOnce({
        settings: { ...newerSettings, defaultScanFolders: [selectedRoot] },
        revision: 9
      });
    const container = document.createElement("div");
    root = createRoot(container);
    act(() => root?.render(createElement(SettingsHarness, { onState: (state) => { latest = state; } })));
    await flushPromises();

    expect(latest?.isLoadingSettings).toBe(true);
    expect(settingsMocks.getSettings).toHaveBeenCalledOnce();
    const saving = latest?.updateSettings({ defaultScanFolders: [selectedRoot] });
    expect(settingsMocks.saveSettings).not.toHaveBeenCalled();
    settingsMocks.getSettings.mockResolvedValueOnce({ settings: newerSettings, revision: 8 });

    loaded.resolve({ settings: DEFAULT_APP_SETTINGS, revision: 7 });
    await saving;
    await flushPromises();

    expect(settingsMocks.saveSettings).toHaveBeenNthCalledWith(1, {
      settings: { ...DEFAULT_APP_SETTINGS, defaultScanFolders: [selectedRoot] },
      expectedRevision: 7
    });
    expect(settingsMocks.saveSettings).toHaveBeenNthCalledWith(2, {
      settings: { ...newerSettings, defaultScanFolders: [selectedRoot] },
      expectedRevision: 8
    });
    expect(latest?.settings.defaultScanFolders).toEqual([selectedRoot]);
    expect(latest?.settings.backgroundIndexOnStartup).toBe(false);
  });

  it("keeps runtime settings at the persisted revision until a save succeeds", async () => {
    settingsMocks.getSettings.mockResolvedValue({ settings: DEFAULT_APP_SETTINGS, revision: 4 });
    const saved = deferred<ReturnType<typeof versionedSettings>>();
    settingsMocks.saveSettings.mockReturnValue(saved.promise);
    const container = document.createElement("div");
    root = createRoot(container);
    act(() => root?.render(createElement(SettingsHarness, { onState: (state) => { latest = state; } })));
    await flushPromises();

    const selectedRoot = createScanRootSetting("C:/OwnerQualification/fixture", "2026-10-10T00:00:00.000Z");
    let saving: Promise<unknown> | undefined;
    act(() => {
      saving = latest?.updateSettingsWithResult({ defaultScanFolders: [selectedRoot] });
    });
    await flushPromises();

    expect(latest?.settings.defaultScanFolders).toEqual([selectedRoot]);
    expect(latest?.persistedSettings.defaultScanFolders).toEqual([]);

    saved.resolve({ settings: { ...DEFAULT_APP_SETTINGS, defaultScanFolders: [selectedRoot] }, revision: 5 });
    await saving;
    await flushPromises();

    expect(latest?.persistedSettings.defaultScanFolders).toEqual([selectedRoot]);
  });

  it("does not expose an unpersisted root to runtime consumers after a failed save", async () => {
    settingsMocks.getSettings.mockResolvedValue({ settings: DEFAULT_APP_SETTINGS, revision: 4 });
    settingsMocks.saveSettings.mockRejectedValue(new Error("settings_save_failure:watcher_runtime_failure"));
    const container = document.createElement("div");
    root = createRoot(container);
    act(() => root?.render(createElement(SettingsHarness, { onState: (state) => { latest = state; } })));
    await flushPromises();

    const selectedRoot = createScanRootSetting("C:/OwnerQualification/fixture", "2026-10-10T00:00:00.000Z");
    let persisted = true;
    await act(async () => {
      persisted = (await latest?.updateSettingsWithResult({ defaultScanFolders: [selectedRoot] }))?.persisted ?? true;
    });

    expect(persisted).toBe(false);
    expect(latest?.settings.defaultScanFolders).toEqual([]);
    expect(latest?.persistedSettings.defaultScanFolders).toEqual([]);
  });

  it("serializes fast partial updates against the latest persisted revision", async () => {
    settingsMocks.getSettings.mockResolvedValue({
      settings: DEFAULT_APP_SETTINGS,
      revision: 10
    });
    settingsMocks.saveSettings
      .mockResolvedValueOnce({
        settings: { ...DEFAULT_APP_SETTINGS, searchHotkey: "Ctrl+J" },
        revision: 11
      })
      .mockResolvedValueOnce({
        settings: { ...DEFAULT_APP_SETTINGS, searchHotkey: "Ctrl+J", restoreRetentionDays: 90 },
        revision: 12
      });
    const container = document.createElement("div");
    root = createRoot(container);
    root.render(createElement(SettingsHarness, { onState: (state) => { latest = state; } }));
    await flushPromises();

    const first = latest?.updateSettings({ searchHotkey: "Ctrl+J" });
    const second = latest?.updateSettings({ restoreRetentionDays: 90 });
    await Promise.all([first, second]);
    await flushPromises();

    expect(settingsMocks.saveSettings).toHaveBeenNthCalledWith(1, {
      settings: { ...DEFAULT_APP_SETTINGS, searchHotkey: "Ctrl+J" },
      expectedRevision: 10
    });
    expect(settingsMocks.saveSettings).toHaveBeenNthCalledWith(2, {
      settings: { ...DEFAULT_APP_SETTINGS, searchHotkey: "Ctrl+J", restoreRetentionDays: 90 },
      expectedRevision: 11
    });
    expect(latest?.settings.restoreRetentionDays).toBe(90);
  });

  it("does not refetch persisted settings when the mounted language changes", async () => {
    settingsMocks.getSettings.mockResolvedValue({
      settings: DEFAULT_APP_SETTINGS,
      revision: 12
    });
    const container = document.createElement("div");
    root = createRoot(container);
    act(() => root?.render(createElement(SettingsHarness, { language: "en", onState: (state) => { latest = state; } })));
    await flushPromises();
    expect(settingsMocks.getSettings).toHaveBeenCalledTimes(1);

    act(() => root?.render(createElement(SettingsHarness, { language: "zh", onState: (state) => { latest = state; } })));
    await flushPromises();
    expect(settingsMocks.getSettings).toHaveBeenCalledTimes(1);
  });
});

function SettingsHarness({ language = "en", onState }: { language?: string; onState: (state: ReturnType<typeof useAppSettings>) => void }) {
  const state = useAppSettings({
    isDatabaseReady: true,
    onError: settingsMocks.onError,
    formatLoadError: () => `${language} load failed`
  });
  onState(state);
  return null;
}

function versionedSettings() {
  return {
    settings: DEFAULT_APP_SETTINGS,
    revision: 0
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

async function flushPromises() {
  await Promise.resolve();
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  await Promise.resolve();
  await Promise.resolve();
}
