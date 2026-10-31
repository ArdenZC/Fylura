import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { activateCommandNavigation, isSortingPreviewShortcut } from "../src/components/CommandModal";
import { createCommandRegistry, executeSpotlightCommand, queryCommandRegistry } from "../src/components/spotlight/commandRegistry";
import { makeTranslator } from "../src/i18n";
import { applySearchNavigation, shouldApplySearchNavigation } from "../src/utils/searchNavigation";
import { projectAcceptedFileLibraryActivation } from "../src/utils/fileLibraryActivation";
import { committedSpotlightInput, completedSpotlightComposition } from "../src/components/spotlight/spotlightComposition";
import { mockGlobalSearchResponseForTests, mockInvokeCommand } from "../src/api/browserMockApi";
import { DEFAULT_SEARCH_HOTKEY, formatHotkeyLabel } from "../src/utils/hotkeys";

describe("spotlight search navigation", () => {
  it("routes the Automation Spotlight command canonically while retaining Rules search keywords", () => {
    const englishCommands = createCommandRegistry(makeTranslator("en"));
    const automation = englishCommands.find((command) => command.id === "automation")!;
    expect(automation.label).toBe("Automation");
    expect(automation.view).toBe("automation");
    expect(queryCommandRegistry("rules", englishCommands).map((command) => command.id)).toContain("automation");

    const chineseCommands = createCommandRegistry(makeTranslator("zh"));
    expect(queryCommandRegistry("规则", chineseCommands).map((command) => command.id)).toContain("automation");

    const setView = vi.fn();
    executeSpotlightCommand(automation, {
      setView,
      requestSettingsSection: vi.fn(),
      onClose: vi.fn()
    });
    expect(setView).toHaveBeenCalledWith("automation");
  });

  it("displays the registered global shortcut for each platform", () => {
    expect(DEFAULT_SEARCH_HOTKEY).toBe("CmdOrCtrl+K");
    expect(formatHotkeyLabel(DEFAULT_SEARCH_HOTKEY, "darwin")).toBe("⌘ K");
    expect(formatHotkeyLabel(DEFAULT_SEARCH_HOTKEY, "win32")).toBe("Ctrl K");
    expect(formatHotkeyLabel(DEFAULT_SEARCH_HOTKEY, "linux")).toBe("Ctrl K");
  });

  it("activates standalone search results through the backend command", async () => {
    const activateSearchResult = vi.fn(async () => {});
    const setView = vi.fn();
    const setSelectedFileId = vi.fn();
    const onClose = vi.fn();

    await activateCommandNavigation({
      standalone: true,
      view: "library",
      fileId: "file-1",
      setView,
      setSelectedFileId,
      onClose,
      activateSearchResult
    });

    expect(activateSearchResult).toHaveBeenCalledWith("library", "file-1", undefined);
    expect(setSelectedFileId).not.toHaveBeenCalled();
    expect(setView).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("carries only fixed settings targets through standalone navigation", async () => {
    const activateSearchResult = vi.fn(async () => {});

    await activateCommandNavigation({
      standalone: true,
      view: "settings",
      fileId: null,
      settingsTarget: "global-index",
      windowSnapshot: { sessionId: 4, revision: 9, phase: "visible_collapsed" },
      setView: vi.fn(),
      setSelectedFileId: vi.fn(),
      onClose: vi.fn(),
      activateSearchResult
    });

    expect(activateSearchResult).toHaveBeenCalledWith(
      "settings",
      null,
      { sessionId: 4, revision: 9, phase: "visible_collapsed" },
      "global-index"
    );
  });

  it("commits one final IME value and never commits composition updates", () => {
    let committed = "";
    const compositionUpdates = ["z", "zh", "zhong"];
    for (const value of compositionUpdates) {
      expect(committedSpotlightInput(value, true, true, 229)).toBeNull();
    }
    committed = completedSpotlightComposition("中");
    expect(committed).toBe("中");
    expect(committedSpotlightInput(committed, false, false, 0)).toBe("中");
  });

  it("keeps no-source global search distinct from an indexed empty result", () => {
    const request = {
      version: 2 as const,
      requestId: "no-source-test",
      query: "missing",
      limit: 20,
      offset: 0
    };
    const noSource = mockGlobalSearchResponseForTests(request, []);
    const indexedEmpty = mockGlobalSearchResponseForTests(request);

    expect(noSource.results).toEqual([]);
    expect(noSource.resultState).toBe("no_source");
    expect(noSource.sourceHealth).toEqual([]);
    expect(indexedEmpty.results).toEqual([]);
    expect(indexedEmpty.resultState).toBe("empty");
    expect(makeTranslator("zh")("globalSearchNoSourcesDesc")).toContain("启用或配置");
    expect(makeTranslator("en")("globalSearchNoSourcesDesc")).toContain("Enable or configure");
  });

  it("keeps in-window command navigation local", async () => {
    const activateSearchResult = vi.fn(async () => {});
    const setView = vi.fn();
    const setSelectedFileId = vi.fn();
    const onClose = vi.fn();

    await activateCommandNavigation({
      standalone: false,
      view: "library",
      fileId: "file-1",
      setView,
      setSelectedFileId,
      onClose,
      activateSearchResult
    });

    expect(setSelectedFileId).toHaveBeenCalledWith("file-1");
    expect(setView).toHaveBeenCalledWith("library");
    expect(onClose).toHaveBeenCalledOnce();
    expect(activateSearchResult).not.toHaveBeenCalled();
  });

  it("projects accepted command file activation into explicit V2 selection and Inspector", async () => {
    const setSelectedFileId = vi.fn();
    const setExplicitSelection = vi.fn();
    const loadDetail = vi.fn(async () => ({ status: "superseded" as const, requestEpoch: 1 }));
    const onClose = vi.fn();

    await activateCommandNavigation({
      standalone: false,
      view: "library",
      fileId: "file-not-on-loaded-page",
      setView: vi.fn(),
      setSelectedFileId,
      activateFileLibraryFile: (fileId) => projectAcceptedFileLibraryActivation(fileId, {
        setExplicitSelection,
        loadDetail
      }),
      onClose
    });

    expect(setSelectedFileId).toHaveBeenCalledWith("file-not-on-loaded-page");
    expect(setExplicitSelection).toHaveBeenCalledWith(["file-not-on-loaded-page"], "file-not-on-loaded-page", -1);
    expect(loadDetail).toHaveBeenCalledWith("file-not-on-loaded-page");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does not create a V2 selection for command-only navigation without a file ID", async () => {
    const activateFileLibraryFile = vi.fn();

    await activateCommandNavigation({
      standalone: false,
      view: "settings",
      fileId: null,
      setView: vi.fn(),
      setSelectedFileId: vi.fn(),
      activateFileLibraryFile,
      onClose: vi.fn()
    });

    expect(activateFileLibraryFile).not.toHaveBeenCalled();
  });

  it("applies search-navigate payloads to the main window state", () => {
    const setView = vi.fn();
    const setSelectedFileId = vi.fn();
    const setExplicitSelection = vi.fn();
    const loadDetail = vi.fn(async () => ({ status: "superseded" as const, requestEpoch: 2 }));
    const activateFileLibraryFile = vi.fn((fileId: string) => projectAcceptedFileLibraryActivation(fileId, {
      setExplicitSelection,
      loadDetail
    }));

    applySearchNavigation({ view: "library", fileId: "file-1" }, setView, setSelectedFileId, undefined, activateFileLibraryFile);
    applySearchNavigation({ view: "preview", fileId: null }, setView, setSelectedFileId, undefined, activateFileLibraryFile);

    expect(setView).toHaveBeenNthCalledWith(1, "library");
    expect(setSelectedFileId).toHaveBeenCalledWith("file-1");
    expect(setExplicitSelection).toHaveBeenCalledWith(["file-1"], "file-1", -1);
    expect(loadDetail).toHaveBeenCalledWith("file-1");
    expect(setView).toHaveBeenNthCalledWith(2, "preview");
    expect(setSelectedFileId).toHaveBeenCalledTimes(1);
    expect(setExplicitSelection).toHaveBeenCalledTimes(1);
    expect(loadDetail).toHaveBeenCalledTimes(1);
  });

  it("normalizes legacy cross-window Rules input without weakening nonce, session, revision or Settings guards", () => {
    const pending = {
      nonce: 17,
      generation: 2,
      view: "scanner" as const,
      selectedFileId: "",
      librarySelection: null,
      libraryFocusedId: "",
      sessionId: 4,
      revision: 9
    };
    const current = {
      view: "scanner" as const,
      selectedFileId: "",
      librarySelection: null,
      libraryFocusedId: ""
    };
    const legacyPayload = { generation: 2, nonce: 17, view: "rules", fileId: null, sessionId: 4, revision: 9 };
    expect(shouldApplySearchNavigation(legacyPayload, pending, current)).toBe(true);

    const setView = vi.fn();
    const setSelectedFileId = vi.fn();
    expect(applySearchNavigation({ view: "automation", fileId: null }, setView, setSelectedFileId)).toBe(true);
    expect(applySearchNavigation(legacyPayload, setView, setSelectedFileId)).toBe(true);
    expect(setView).toHaveBeenNthCalledWith(1, "automation");
    expect(setView).toHaveBeenNthCalledWith(2, "automation");

    expect(shouldApplySearchNavigation({ ...legacyPayload, nonce: 16 }, pending, current)).toBe(false);
    expect(shouldApplySearchNavigation({ ...legacyPayload, sessionId: 3 }, pending, current)).toBe(false);
    expect(shouldApplySearchNavigation({ ...legacyPayload, revision: 8 }, pending, current)).toBe(false);
    expect(applySearchNavigation({ ...legacyPayload, settingsTarget: "ai" }, setView, setSelectedFileId)).toBe(false);
    expect(setView).toHaveBeenCalledTimes(2);
    expect(setSelectedFileId).not.toHaveBeenCalled();
  });

  it("applies fixed standalone settings targets and rejects illegal targets", () => {
    const setView = vi.fn();
    const setSelectedFileId = vi.fn();
    const requestSettingsSection = vi.fn();

    expect(applySearchNavigation(
      {
        view: "settings",
        fileId: null,
        nonce: 5,
        sessionId: 2,
        revision: 8,
        settingsTarget: "global-index"
      },
      setView,
      setSelectedFileId,
      requestSettingsSection
    )).toBe(true);
    expect(requestSettingsSection).toHaveBeenCalledWith("settings-global-index");
    expect(applySearchNavigation(
      { view: "settings", fileId: null, settingsTarget: "not-a-settings-section" },
      setView,
      setSelectedFileId,
      requestSettingsSection
    )).toBe(false);
    expect(requestSettingsSection).toHaveBeenCalledOnce();
  });

  it("keeps the browser mock wire-compatible without faking native navigation", async () => {
    await expect(mockInvokeCommand("activate_search_result", {
      request: {
        sessionId: 2,
        expectedRevision: 8,
        view: "settings",
        fileId: null,
        settingsTarget: "global-index"
      }
    })).resolves.toBeUndefined();
    await expect(mockInvokeCommand("activate_search_result", {
      request: { view: "settings", settingsTarget: "arbitrary-selector" }
    })).rejects.toThrow("search_navigation_settings_target_invalid");
  });

  it("uses the independent global index for command and standalone Spotlight results", () => {
    const commandModal = readFileSync(resolve("src/components/CommandModal.tsx"), "utf8");
    const appShell = readFileSync(resolve("src/components/AppShell.tsx"), "utf8");
    const runtimeProviders = (readFileSync(resolve("src/components/AppRuntimeProviders.tsx"), "utf8") + readFileSync(resolve("src/hooks/useSearchNavigationHandoff.ts"), "utf8")).replace(/\r\n/g, "\n");

    expect(commandModal).toContain("const SEARCH_RESULT_LIMIT = 80");
    expect(commandModal).toContain("tauriApi.searchGlobalEntries(request)");
    expect(commandModal).toContain("queryControllerRef.current.nextRequest(committedTrimmedSearch, SEARCH_RESULT_LIMIT)");
    expect(commandModal).toContain("committedSpotlightInput");
    expect(commandModal).toContain("queryControllerRef.current.accepts(response)");
    expect(commandModal).toContain("mergeSpotlightResults(currentGlobalResults, commandResults)");
    expect(commandModal).toContain("filesForCurrentQuery(trimmedSearch, globalResultState.query, globalResultState.results)");
    expect(commandModal).toContain('setGlobalResultState({ query: committedTrimmedSearch, results: [] })');
    expect(commandModal).toContain("queryCommandRegistry(searchableQuery, commandRegistry)");
    expect(commandModal).toContain("groupSpotlightResults(visibleResults, t)");
    expect(commandModal).not.toContain("results.slice(0, 12)");
    expect(commandModal).toContain("scrollIntoView({ block: \"nearest\" })");
    expect(commandModal).toContain("max-h-[50vh] overflow-y-auto p-2");
    expect(commandModal).not.toContain("tauriApi.getPagedFiles(12, 0, trimmedSearch");
    expect(commandModal).toContain("tauriApi.openGlobalSearchResult(entry.id)");
    expect(commandModal).toContain("tauriApi.revealGlobalSearchResult(activeResult.entry.id)");
    expect(commandModal).not.toContain("searchScope");
    expect(appShell).not.toContain("resolveEffectiveSearchScope");
    expect(appShell).not.toContain("searchScope={effectiveSearchScope}");
    expect(appShell).toContain("useFileLibrarySelectionStore");
    expect(appShell).toContain("useFileLibraryInspectorStore");
    expect(appShell).toContain("activateFileLibraryFile={activateFileLibraryFile}");
    expect(commandModal).toContain("activateFileLibraryFile?.(fileId)");
    expect(appShell).toContain("projectAcceptedFileLibraryActivation");
    expect(runtimeProviders).toMatch(/requestSettingsSection,\s+activateFileLibraryFile/);
  });

  it("acknowledges Rust-owned main-window navigation readiness", () => {
    const runtimeProviders = (readFileSync(resolve("src/components/AppRuntimeProviders.tsx"), "utf8") + readFileSync(resolve("src/hooks/useSearchNavigationHandoff.ts"), "utf8"));

    expect(runtimeProviders).toContain("tauriApi.onMainWindowReadyRequest");
    expect(runtimeProviders).toContain("sessionId = null, revision = null");
    expect(runtimeProviders).toContain("tauriApi.acknowledgeMainWindowReady(nonce)");
    expect(runtimeProviders).toContain("tauriApi.markMainWindowReady(true)");
    expect(runtimeProviders).not.toContain("onGlobalSearchRequested");
  });

  it("rejects late navigation after the user changes main-window state", () => {
    const pendingSelection = { kind: "explicit" as const, fileIds: ["file-existing"] };
    const pending = {
      nonce: 9,
      generation: 2,
      view: "scanner" as const,
      selectedFileId: "",
      librarySelection: pendingSelection,
      libraryFocusedId: "file-existing",
      sessionId: 4,
      revision: 12
    };
    const unchanged = {
      view: "scanner" as const,
      selectedFileId: "",
      librarySelection: pendingSelection,
      libraryFocusedId: "file-existing"
    };
    const setView = vi.fn();
    const setSelectedFileId = vi.fn();
    const setExplicitSelection = vi.fn();
    const loadDetail = vi.fn(async () => ({ status: "superseded" as const, requestEpoch: 3 }));
    const activateFileLibraryFile = vi.fn((fileId: string) => projectAcceptedFileLibraryActivation(fileId, {
      setExplicitSelection,
      loadDetail
    }));
    expect(shouldApplySearchNavigation(
      { generation: 2, nonce: 9, view: "library", fileId: "file-1", sessionId: 4, revision: 12 },
      pending,
      unchanged
    )).toBe(true);
    expect(shouldApplySearchNavigation(
      { generation: 2, nonce: 9, view: "library", fileId: "file-1", sessionId: 4, revision: 12 },
      pending,
      { ...unchanged, view: "settings" }
    )).toBe(false);
    expect(shouldApplySearchNavigation(
      { generation: 2, nonce: 9, view: "library", fileId: "file-1", sessionId: 4, revision: 12 },
      pending,
      { ...unchanged, librarySelection: { kind: "explicit", fileIds: ["file-existing"] } }
    )).toBe(false);
    expect(shouldApplySearchNavigation(
      { generation: 2, nonce: 9, view: "library", fileId: "file-1", sessionId: 4, revision: 12 },
      pending,
      { ...unchanged, libraryFocusedId: "file-other" }
    )).toBe(false);
    expect(shouldApplySearchNavigation(
      { generation: 2, nonce: 8, view: "library", fileId: "file-1", sessionId: 4, revision: 12 },
      pending,
      unchanged
    )).toBe(false);
    expect(shouldApplySearchNavigation(
      { generation: 2, nonce: 9, view: "settings", fileId: null, sessionId: 4, revision: 12, settingsTarget: "not-a-settings-section" },
      pending,
      unchanged
    )).toBe(false);
    expect(shouldApplySearchNavigation(
      { generation: 2, nonce: 9, view: "library", fileId: "file-1", sessionId: 3, revision: 12 },
      pending,
      unchanged
    )).toBe(false);
    expect(shouldApplySearchNavigation(
      { generation: 2, nonce: 9, view: "library", fileId: "file-1", sessionId: 4, revision: 11 },
      pending,
      unchanged
    )).toBe(false);
    const stalePayload = { generation: 2, nonce: 8, view: "library", fileId: "file-1", sessionId: 4, revision: 12 };
    if (shouldApplySearchNavigation(stalePayload, pending, unchanged)) {
      applySearchNavigation(stalePayload, setView, setSelectedFileId, undefined, activateFileLibraryFile);
    }
    expect(setView).not.toHaveBeenCalled();
    expect(setSelectedFileId).not.toHaveBeenCalled();
    expect(activateFileLibraryFile).not.toHaveBeenCalled();
    expect(setExplicitSelection).not.toHaveBeenCalled();
    expect(loadDetail).not.toHaveBeenCalled();
  });

  it("keeps the external activation bridge stateless and scoped to existing V2 actions", () => {
    const bridge = readFileSync(resolve("src/utils/fileLibraryActivation.ts"), "utf8");

    expect(bridge).toContain("setExplicitSelection([fileId], fileId, -1)");
    expect(bridge).toContain("void bridge.loadDetail(fileId)");
    expect(bridge).not.toContain("create(");
    expect(bridge).not.toContain("localStorage");
    expect(bridge).not.toContain("listen");
  });

  it("uses folder-aware wording, plural-safe counts, and neutral indexed-entry icons", () => {
    const commandModal = readFileSync(resolve("src/components/CommandModal.tsx"), "utf8");
    const en = makeTranslator("en");
    expect(en("globalSearch")).toBe("Search folders, files, actions, or settings");
    expect(commandModal).toContain("formatCount(t, visibleResults.length");
    expect(commandModal).toContain("entry.isDirectory ? <Folder size={20} /> : <FileIcon size={20} />");
    expect(commandModal).not.toContain("<FileTypeIcon");
  });

  it("fails closed for an invalid runtime view payload", () => {
    const setView = vi.fn();
    const setSelectedFileId = vi.fn();
    const activateFileLibraryFile = vi.fn();

    expect(applySearchNavigation(
      { view: "destructive-unknown-view", fileId: "file-1" },
      setView,
      setSelectedFileId,
      undefined,
      activateFileLibraryFile
    )).toBe(false);

    expect(setView).not.toHaveBeenCalled();
    expect(setSelectedFileId).not.toHaveBeenCalled();
    expect(activateFileLibraryFile).not.toHaveBeenCalled();
  });

  it("keeps Tab available for focus movement and uses primary-key shortcuts for sorting preview", () => {
    const keyEvent = (key: string, overrides: Partial<KeyboardEvent> = {}) => ({
      key,
      ctrlKey: false,
      metaKey: false,
      altKey: false,
      shiftKey: false,
      ...overrides
    } as KeyboardEvent);

    expect(isSortingPreviewShortcut(keyEvent("Tab"))).toBe(false);
    expect(isSortingPreviewShortcut(keyEvent("Enter"))).toBe(false);
    expect(isSortingPreviewShortcut(keyEvent("Enter", { ctrlKey: true }))).toBe(true);
    expect(isSortingPreviewShortcut(keyEvent("Enter", { metaKey: true }))).toBe(true);
    expect(isSortingPreviewShortcut(keyEvent("p", { ctrlKey: true }))).toBe(true);
    expect(isSortingPreviewShortcut(keyEvent("P", { metaKey: true }))).toBe(true);
    expect(isSortingPreviewShortcut(keyEvent("P", { ctrlKey: true, shiftKey: true }))).toBe(false);
  });

  it("configures the global search window as a transparent spotlight surface", () => {
    const appControl = readFileSync(resolve("src-tauri/src/app_control.rs"), "utf8");
    const cargoToml = readFileSync(resolve("src-tauri/Cargo.toml"), "utf8");
    const tauriConfig = readFileSync(resolve("src-tauri/tauri.conf.json"), "utf8");
    const appShell = readFileSync(resolve("src/components/AppShell.tsx"), "utf8");
    const tauriApi = readFileSync(resolve("src/api/windowApi.ts"), "utf8");
    const mainRs = readFileSync(resolve("src-tauri/src/main.rs"), "utf8");
    const main = readFileSync(resolve("src/main.tsx"), "utf8");
    const styles = readFileSync(resolve("src/styles.css"), "utf8");

    const setupSearchWindow = appControl.slice(
      appControl.indexOf("fn create_search_window"),
      appControl.indexOf("pub fn setup_global_search_shortcut")
    );

    expect(setupSearchWindow).toContain(".transparent(true)");
    expect(appControl).toContain("SEARCH_WINDOW_WIDTH: f64 = 820.0");
    expect(appControl).toContain("SEARCH_WINDOW_COLLAPSED_HEIGHT: f64 = 160.0");
    expect(appControl).toContain("SEARCH_WINDOW_EXPANDED_HEIGHT: f64 = 660.0");
    expect(appControl).not.toContain("SEARCH_WINDOW_HEIGHT: f64 = 320.0");
    expect(appControl).toContain("pub fn resize_search_window<R: Runtime>");
    expect(appControl).toContain("expanded: bool");
    expect(appControl).toContain('.ok_or_else(|| "search_window_missing".to_string())?');
    expect(appControl).toContain("SEARCH_WINDOW_EXPANDED_HEIGHT");
    expect(appControl).toContain("window.center().map_err(|error| error.to_string())?");
    expect(setupSearchWindow).toContain(".inner_size(SEARCH_WINDOW_WIDTH, SEARCH_WINDOW_COLLAPSED_HEIGHT)");
    expect(setupSearchWindow).toContain(".min_inner_size(SEARCH_WINDOW_WIDTH, SEARCH_WINDOW_COLLAPSED_HEIGHT)");
    expect(setupSearchWindow).toContain(".max_inner_size(SEARCH_WINDOW_WIDTH, SEARCH_WINDOW_EXPANDED_HEIGHT)");
    expect(setupSearchWindow).toContain(".shadow(false)");
    expect(appControl).toContain(".set_size(Size::Logical(LogicalSize");
    expect(appControl).toContain("height: if expanded");
    expect(appControl).toContain("SEARCH_WINDOW_EXPANDED_HEIGHT");
    expect(appControl).toContain("SEARCH_WINDOW_COLLAPSED_HEIGHT");
    expect(setupSearchWindow).not.toContain("target_os = \"windows\", target_os = \"linux\"");
    expect(cargoToml).toContain("\"tauri/macos-private-api\"");
    expect(tauriConfig).toContain("\"macOSPrivateApi\": true");
    expect(setupSearchWindow).toContain(".decorations(false)");
    expect(setupSearchWindow).toContain(".resizable(false)");
    expect(setupSearchWindow).toContain(".skip_taskbar(true)");
    expect(setupSearchWindow).toContain(".always_on_top(true)");
    expect(mainRs).toContain("fylura_tauri::app_control::resize_search_window");
    expect(mainRs).toContain("fylura_tauri::app_control::hide_search_window_command");
    expect(mainRs).not.toContain("setup_search_window");
    expect(tauriApi).toContain("resizeSearchWindow(snapshot: SearchWindowSnapshot, expanded: boolean)");
    expect(tauriApi).toContain('invokeCommand<SearchWindowSnapshot>("resize_search_window"');
    expect(appShell).toContain("const searchWindowRoot =");
    expect(appShell).toContain("bg-transparent");
    expect(appShell).toContain("h-full w-full");
    expect(appShell).not.toContain("h-screen w-screen");
    expect(appShell).toContain("<div className={searchWindowRoot}>");
    const commandModal = readFileSync(resolve("src/components/CommandModal.tsx"), "utf8");
    expect(commandModal).toContain("standaloneSearchWindowCollapsedHeight = 160");
    expect(commandModal).toContain("standaloneSearchWindowExpandedHeight = 660");
    expect(commandModal).toContain("max-h-[50vh] overflow-y-auto p-2");
    expect(commandModal).toContain("const isStandaloneCollapsed =");
    expect(commandModal).toContain("tauriApi.resizeSearchWindow(searchWindowSnapshot, expanded)");
    expect(commandModal).toContain("tauriApi.hideSearchWindow(snapshot)");
    expect(commandModal).toContain("tauriApi.onSearchWindowState(updateSearchWindowSnapshot)");
    expect(commandModal).toContain("commandShellBase");
    expect(commandModal).toContain("commandShellCollapsed");
    expect(commandModal).toContain("commandShellExpanded");
    expect(commandModal).toContain("h-16 w-full max-w-[720px] rounded-full");
    expect(commandModal).toContain("const shouldShowIdleState = !standalone && !trimmedSearch");
    expect(commandModal).not.toContain("pt-[9vh]");
    expect(commandModal).not.toContain("px-5 pt-2");
    expect(commandModal).not.toContain("@tauri-apps/api/window");
    expect(main).toContain("search-window-root");
    expect(main).not.toContain("search-window-page");
    expect(styles).toContain("html.search-window-root");
    expect(styles).not.toContain("search-window-page");
    expect(styles).toContain("min-width: 0");
    expect(styles).toContain("background: transparent");
  });
});
