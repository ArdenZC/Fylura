import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

describe("background indexer", () => {
  it("queues enabled scan and search roots without taking over the foreground scanner", () => {
    const runtimeProviders = read("src/components/AppRuntimeProviders.tsx");
    const indexerStore = read("src/store/useBackgroundIndexerStore.ts");

    expect(runtimeProviders).toContain("useBackgroundIndexerStore");
    expect(runtimeProviders).toContain("enqueueBackgroundIndexRoots");
    expect(runtimeProviders).toContain("enabledScanRootPaths(persistedSettings.defaultScanFolders)");
    expect(runtimeProviders).toContain("enabledSearchRootPaths(persistedSettings.customSearchRoots)");
    expect(runtimeProviders).toContain("if (isSearchMode || isLoadingSettings) return");
    expect(runtimeProviders).toContain("persistedSettings.backgroundIndexOnStartup === false");
    expect(runtimeProviders).toContain("const backgroundIndexRoots = useMemo");
    expect(runtimeProviders).toContain("const backgroundIndexRootSignature = useMemo");
    expect(runtimeProviders).toContain("backgroundIndexRootSignature");
    expect(runtimeProviders).not.toContain("appSettings.searchHotkey,\n    appSettings.customSearchRoots");
    expect(indexerStore).toContain("pendingRoots: string[]");
    expect(indexerStore).toContain("currentRoot: string | null");
    expect(indexerStore).toContain("isBackgroundIndexing: boolean");
    expect(indexerStore).toContain("failedRoots: BackgroundIndexFailure[]");
    expect(indexerStore).toContain("completedRoots: string[]");
    expect(indexerStore).toContain("useScanManagerStore.getState().isScanning");
    expect(indexerStore).toContain("await tauriApi.startManagedScan(request)");
    expect(indexerStore).toContain("await waitForManagedBackgroundSession(request, start)");
    expect(indexerStore).toContain("await tauriApi.cancelScanRun(jobId)");
    expect(indexerStore).toContain("const knownRoots = new Set");
    expect(indexerStore).toContain("...state.pendingRoots.map(normalizeRoot)");
    expect(indexerStore).toContain("state.currentRoot ? [normalizeRoot(state.currentRoot)]");
    expect(indexerStore).toContain("recentlyIndexedRoots");
    expect(indexerStore).toContain("markRecentlyIndexedRoot(root)");
    expect(indexerStore).toContain("options?.force");
    expect(indexerStore).toContain("state.completedRoots.map(normalizeRoot)");
    expect(indexerStore).toContain("useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery)");
    expect(indexerStore).not.toContain("showSuccess");
    expect(indexerStore).not.toContain("showError");
    expect(indexerStore).not.toContain("setView");
  });

  it("surfaces quiet local-index status in settings copy", () => {
    const settingsView = read("src/views/settings/SettingsView.tsx");
    const settingsSurface = [
      settingsView,
      read("src/views/settings/sections/GeneralSettingsSection.tsx"),
      read("src/views/settings/sections/GlobalSearchSettingsSection.tsx")
    ].join("\n");
    const zh = makeTranslator("zh");
    const en = makeTranslator("en");

    expect(zh("settingsSearchDesc")).toContain("本地索引");
    expect(zh("searchScopeDoesNotChangeLibrary")).toContain("后台建立本地索引");
    expect(zh("backgroundIndexQueued")).toBe("已加入后台索引队列");
    expect(zh("backgroundIndexOnStartup")).toBe("启动后静默更新索引");
    expect(zh("commandCustomRootsNoResults")).toContain("指定文件夹需要先建立本地索引");
    expect(en("settingsSearchDesc")).toContain("local-index");
    expect(en("searchScopeSettingsDesc")).toContain("local SQLite index");
    expect(en("searchScopeAllIndexedLabel")).toBe("Search scope: all indexed files");
    expect(en("searchLocalIndexBoundary")).toContain("local index");
    expect(en("backgroundIndexOnStartup")).toBe("Update index in background on startup");
    expect(settingsSurface).toContain("pendingBackgroundRoots");
    expect(settingsSurface).toContain("currentBackgroundRoot");
    expect(settingsSurface).toContain("isBackgroundIndexing");
    expect(settingsSurface).toContain('t("backgroundIndexingTitle")');
    expect(settingsSurface).toContain('t("backgroundIndexingQueue")');
    expect(settingsSurface).toContain('t("backgroundIndexOnStartup")');
    expect(settingsSurface).toContain('t("searchLocalIndexBoundary")');
    expect(settingsSurface).toContain("enqueueBackgroundIndexRoot(path)");
    expect(settingsSurface).toContain("if (enabled) enqueueBackgroundIndexRoot(root.path, { force: true })");
    expect(settingsSurface).toContain("enqueueBackgroundIndexRoot(root.path, { force: true })");
    expect(settingsSurface).toContain("indexSearchRootNow");
  });

  it("keeps Spotlight independent from the legacy custom-root index queue", () => {
    const commandModal = read("src/components/CommandModal.tsx");

    expect(commandModal).toContain("tauriApi.searchGlobalEntries(request)");
    expect(commandModal).not.toContain("isCustomRootNoResults");
    expect(commandModal).not.toContain("enqueueBackgroundIndexRoots");
    expect(commandModal).not.toContain("searchScope?.kind === \"roots\"");
  });

  it("keeps watcher roots in sync with custom search roots", () => {
    const watcher = read("src-tauri/src/watcher.rs");

    expect(watcher).toContain("SearchRootSetting");
    expect(watcher).toContain("watch_paths_from_search_roots");
    expect(watcher).toContain("watch_paths_from_settings");
    expect(watcher).toContain("settings.custom_search_roots");
    expect(watcher).toContain("watch_paths_include_enabled_custom_search_roots");
  });
});
