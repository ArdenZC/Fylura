import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const root = process.cwd();

function read(relativePath: string) {
  return readFileSync(join(root, relativePath), "utf8");
}

describe("app render architecture", () => {
  it("keeps high-frequency state and business singletons out of App.tsx", () => {
    const app = read("src/App.tsx");

    expect(app).not.toContain("searchQuery");
    expect(app).not.toContain("toast");
    expect(app).not.toContain("useFileLibrary(");
    expect(app).not.toContain("useScanManager(");
    expect(app).not.toContain("useOperationQueue(");
    expect(app).not.toContain("FileLibraryProvider");
    expect(app).not.toContain("ScanProvider");
    expect(app).not.toContain("OperationQueueProvider");
  });

  it("keeps the browser mock out of the production Tauri API dependency edge", () => {
    const tauriApi = read("src/api/tauriApi.ts");
    const apiCore = read("src/api/core.ts");
    const contentSheet = read("src/views/vault/components/ContentUnderstandingSheet.tsx");
    const ruleProposal = read("src/views/rules/RuleProposalWorkspace.tsx");
    expect(tauriApi).not.toContain('from "./browserMockApi"');
    expect(tauriApi).not.toContain("@tauri-apps/api/core");
    expect(apiCore).toContain('import("./browserMockApi")');
    expect(apiCore).toContain("import.meta.env.DEV");
    expect(contentSheet).not.toContain("api/browserMockApi");
    expect(ruleProposal).not.toContain("api/browserMockApi");
  });

  it("keeps the Tauri facade thin and domain adapters behind shared core", () => {
    const facade = read("src/api/tauriApi.ts");
    const apiCore = read("src/api/core.ts");
    const domainAdapters = [
      "aiApi.ts",
      "analysisApi.ts",
      "cleanupApi.ts",
      "contentApi.ts",
      "dedupeApi.ts",
      "globalSearchApi.ts",
      "libraryApi.ts",
      "operationApi.ts",
      "organizationApi.ts",
      "rulesApi.ts",
      "runtimeApi.ts",
      "scanApi.ts",
      "settingsApi.ts",
      "windowApi.ts"
    ].map((name) => read(`src/api/${name}`));

    expect(facade).toContain("export const tauriApi");
    expect(facade).not.toContain("invokeCommand(");
    expect(facade).not.toContain("listenTo(");
    expect(apiCore).toContain("export async function invokeCommand");
    expect(apiCore).toContain("export async function listenTo");
    for (const adapter of domainAdapters) {
      expect(adapter).not.toContain('@tauri-apps/api/core');
      expect(adapter).not.toMatch(/import\s+\{[^}]*\b(?:invoke|listen)\b/);
    }
  });

  it("keeps Settings and Vault domain effects behind controllers", () => {
    const settings = read("src/views/settings/SettingsView.tsx");
    const settingsNavigation = read("src/views/settings/controllers/useSettingsNavigationController.ts");
    const settingsGlobalIndex = read("src/views/settings/controllers/useSettingsGlobalIndexController.ts");
    const vault = read("src/views/vault/VaultView.tsx");
    const vaultQuery = read("src/views/vault/controllers/useVaultQueryController.ts");

    expect(settings).toContain("useSettingsNavigationController");
    expect(settings).toContain("useSettingsGlobalIndexController");
    expect(settingsNavigation).toContain("SETTINGS_SECTION_EVENT");
    expect(settingsGlobalIndex).toContain("getGlobalIndexStatus");
    expect(settingsGlobalIndex).toContain("useEffect(() => {");
    expect(vault).toContain("useVaultQueryController");
    expect(vault).not.toContain("resolveLegacyLibraryScope(legacyScope)");
    expect(vaultQuery).toContain("resolveLegacyLibraryScope");
    expect(vaultQuery).toContain("loadFirstPage");
  });

  it("uses Zustand stores instead of React context for file, scan, and operation queues", () => {
    const contexts = read("src/contexts/AppContexts.tsx");
    const fileLibraryStore = read("src/store/useFileLibraryStore.ts");
    const scanStore = read("src/store/useScanManagerStore.ts");
    const operationStore = read("src/store/useOperationQueueStore.ts");

    expect(contexts).not.toContain("FileLibraryProvider");
    expect(contexts).not.toContain("ScanProvider");
    expect(contexts).not.toContain("OperationQueueProvider");
    expect(fileLibraryStore).toContain("create<FileLibraryStore>");
    expect(scanStore).toContain("create<ScanManagerStore>");
    expect(operationStore).toContain("create<OperationQueueStore>");
  });

  it("allows folder picking through the dialog open permission only", () => {
    const capability = JSON.parse(read("src-tauri/capabilities/default.json")) as {
      permissions: string[];
    };

    expect(capability.permissions).toContain("dialog:allow-open");
    expect(capability.permissions).not.toContain("dialog:allow-save");
  });

  it("keeps scanner totals and vault filters tied to their real state", () => {
    const scanner = read("src/views/scanner/ScannerView.tsx");
    const overviewModel = read("src/views/overview/overviewModel.ts");
    const vault = read("src/views/vault/VaultView.tsx");
    const vaultQuery = read("src/views/vault/controllers/useVaultQueryController.ts");
    const fileLibraryStore = read("src/store/useFileLibraryStore.ts");

    expect(scanner).toContain("buildOverviewSummary(stats, overviewRoots, t, language)");
    expect(overviewModel).toContain("stats.totalSize");
    expect(overviewModel).toContain("stats.totalFiles");
    expect(scanner).not.toContain("files.reduce((sum, file) => sum + file.size");
    expect(vault).not.toContain('useState<LibraryFilter>("all")');
    expect(fileLibraryStore).toContain("libraryFilter: LibraryFilter");
    expect(fileLibraryStore).toContain("setLibraryFilter");
    expect(vault).toContain("useFileLibraryResultStore");
    expect(vault).toContain("useVaultQueryController");
    expect(vaultQuery).toContain("setQuerySpec(spec)");
    expect(vaultQuery).toContain("void loadFirstPage()");
    expect(vaultQuery).toContain("resolveLegacyLibraryScope");
    expect(read("src/store/useFileLibraryV2Store.ts")).toContain("queryFileLibraryV2");
    expect(vault).not.toContain("setSearchQuery(filter.key)");
  });

  it("keeps the document language aligned with the active UI language", () => {
    const runtimeProviders = read("src/components/AppRuntimeProviders.tsx");

    expect(runtimeProviders).toContain("document.documentElement.lang");
    expect(runtimeProviders).toContain('language === "zh" ? "zh-CN" : "en"');
  });

  it("does not rebuild operation previews from the current paged library rows", () => {
    const runtimeProviders = read("src/components/AppRuntimeProviders.tsx");
    const bootstrapper = runtimeProviders.slice(
      runtimeProviders.indexOf("function StoreRuntimeBootstrapper"),
      runtimeProviders.indexOf("function arraysEqual")
    );

    expect(bootstrapper).not.toContain("libraryPage.files");
    expect(bootstrapper).not.toContain("syncPreviews(files)");
  });

  it("uses the durable plan and existing managed-AI adapter instead of the legacy preview walk", () => {
    const view = read("src/views/organize/OrganizeSuggestionsView.tsx");
    const store = [
      read("src/store/useOrganizationPlanStore.ts"),
      read("src/store/organizationPlan/planMutations.ts")
    ].join("\n");
    const organization = read("src-tauri/src/db/queries/organization/mod.rs");
    expect(view).toContain("useOrganizationPlanStore");
    expect(view).toContain("analyzeMissing");
    expect(view).toContain("createDryRun");
    expect(view).not.toContain("useOperationQueueStore");
    expect(view).not.toContain("loadOrganizeQueue");
    expect(store).toContain("analyzeOrganizationPlanItems");
    expect(organization).toContain("enqueue_managed_ai_for_library_files");
    expect(organization).not.toContain("classify_files_with_ai");
    expect(view).not.toMatch(/temperature|top_p|endpoint|modelName/i);
  });

  it("describes AI batch size as per-request and exposes cleanup AI settings", () => {
    const settings = read("src/views/settings/SettingsView.tsx");
    const i18n = read("src/i18n/dictionary.ts");
    const browserMock = read("src/api/browserMockApi.ts");

    expect(settings).toContain('description={t("aiBatchSizeDesc")}');
    expect(settings).toContain('label={t("aiConcurrencyLabel")}');
    expect(settings).toContain("AI_CLASSIFICATION_LABEL_KEYS");
    expect(settings).toContain("t(AI_CLASSIFICATION_LABEL_KEYS[presetId])");
    expect(settings).toContain('description={t("aiLearnedRulesDesc")}');
    expect(settings).toContain('label={t("aiLegacyRulesLabel")}');
    expect(settings).toContain('description={t("aiLegacyRulesDesc")}');
    expect(settings).toContain('label={t("aiCleanupEnabledLabel")}');
    expect(settings).toContain('description={t("aiCleanupEnabledDesc")}');
    expect(settings).toContain('label={t("aiCleanupLocalConsentLabel")}');
    expect(settings).toContain('description={t("aiCleanupLocalConsentDesc")}');
    expect(settings).toContain('label={t("aiCleanupCloudConsentLabel")}');
    expect(settings).toContain('description={t("aiCleanupCloudConsentDesc")}');
    expect(i18n).toContain("DeepSeek / 国产模型建议 10");
    expect(i18n).toContain("AI 分类并发数");
    expect(i18n).toContain("AI-first 模式下建议关闭");
    expect(i18n).toContain("AI 空间清理分析只增强候选项的风险说明和建议，不会直接删除文件，也不会绕过 Safe Trash。");
    expect(settings).toContain("cleanupAiEnabled: true");
    expect(settings).toContain("cleanupLocalAiAllowed: false");
    expect(settings).toContain("cleanupCloudAiAllowed: false");
    expect(browserMock).toContain("cleanupAiEnabled: true");
    expect(browserMock).toContain("cleanupLocalAiAllowed: false");
    expect(browserMock).toContain("cleanupCloudAiAllowed: false");
  });

  it("keeps automatic rule execution behind explicit confirmation and the current safety boundary", () => {
    const rulesView = read("src/views/rules/RulesView.tsx");
    const i18n = read("src/i18n/dictionary.ts");

    expect(rulesView).toContain('setConfirmation({ kind: "run" })');
    expect(rulesView).toContain("ConfirmDialog");
    expect(i18n).toContain("兼容性策略只更新旧分类展示字段");
    expect(i18n).toContain("不会生成新的 Managed AI 语义建议");
    expect(i18n).toContain("不会直接移动、重命名、删除或覆盖文件");
    expect(i18n).toContain("执行仍需进入预览确认");
  });

  it("does not register main-window runtime side effects in search mode", () => {
    const runtimeProviders = read("src/components/AppRuntimeProviders.tsx");
    const fsWatcher = read("src/hooks/useFsWatcher.ts");
    const bootstrapper = runtimeProviders.slice(
      runtimeProviders.indexOf("function StoreRuntimeBootstrapper"),
      runtimeProviders.indexOf("function arraysEqual")
    );
    const hotkeyFailureIndex = runtimeProviders.indexOf("tauriApi.onGlobalHotkeyRegistrationFailed");
    const searchNavigateHandler = read("src/hooks/useSearchNavigationHandoff.ts");
    expect(read("src/components/AppShell.tsx")).toContain("{!isSearchMode && <SearchLibraryHandoff />}");
    const hotkeyFailureHandler = runtimeProviders.slice(
      runtimeProviders.lastIndexOf("useEffect", hotkeyFailureIndex),
      runtimeProviders.indexOf("const setCloseBehavior")
    );

    expect(fsWatcher).toContain("enabled?: boolean");
    expect(runtimeProviders).toContain("useFsWatcher({");
    expect(runtimeProviders).toContain("enabled: !isSearchMode");
    expect(runtimeProviders).toContain("<StoreRuntimeBootstrapper enabled={!isSearchMode} />");
    expect(bootstrapper).toContain("enabled: boolean");
    expect(bootstrapper).toContain("if (!enabled) return");
    expect(searchNavigateHandler).toContain("if (isSearchMode) return");
    expect(hotkeyFailureHandler).toContain("if (isSearchMode) return");
  });

  it("gates rule persistence in search mode", () => {
    const runtimeProviders = read("src/components/AppRuntimeProviders.tsx");
    const rulePersistence = read("src/hooks/useRulePersistence.ts");
    const useRulePersistenceCall = runtimeProviders.slice(
      runtimeProviders.indexOf("useRulePersistence({"),
      runtimeProviders.indexOf("useFsWatcher({")
    );
    const rulePersistenceEffect = rulePersistence.slice(
      rulePersistence.indexOf("useEffect(() => {"),
      rulePersistence.indexOf("async function hydrateRules")
    );

    expect(rulePersistence).toContain("enabled?: boolean");
    expect(rulePersistence).toContain("enabled = true");
    expect(rulePersistenceEffect).toContain("if (!enabled || !isDatabaseReady || hasHydrated.current) return");
    expect(useRulePersistenceCall).toContain("enabled: !isSearchMode");
    expect(useRulePersistenceCall).toContain("hydrateUserRulesFromSQLite");
  });

  it("reapplies changed rules only from an explicit RulesView action", () => {
    const rulesView = read("src/views/rules/RulesView.tsx");
    const runtimeProviders = read("src/components/AppRuntimeProviders.tsx");
    const saveRule = runtimeProviders.slice(
      runtimeProviders.indexOf("const saveRule"),
      runtimeProviders.indexOf("const toggleRuleEnabled")
    );

    expect(rulesView).toContain("reapplyRulesToCurrentScope");
    expect(rulesView).toContain('"all_changed_or_rule_changed"');
    expect(saveRule).not.toContain("executeRulesForScope");
  });

  it("uses shared UI primitives for the shell frame and scanner entry experience", () => {
    const appShell = read("src/components/AppShell.tsx");
    const scanner = read("src/views/scanner/ScannerView.tsx");

    expect(appShell).toContain("PageHeader");
    expect(appShell).toContain("pageFrame");
    expect(appShell).toContain("viewStage");
    expect(appShell).not.toContain("h-[calc(");
    expect(appShell).not.toContain("cn(pageBody");
    // Overview owns content layout; the App Shell owns the workspace heading.
    expect(scanner).not.toContain("PageHeader");
    expect(scanner).toContain("OverviewPriorityTask");
    expect(scanner).toContain("OverviewSpaceSummary");
    expect(scanner).toContain("OverviewRecentActivityList");
    expect(scanner).toContain("OverviewBackgroundTaskList");
    expect(scanner).toContain("pageSurface");
    expect(scanner).not.toContain("ScannerSummaryChip");
    expect(scanner).not.toContain("ScannerDisk");
    expect(scanner).not.toContain("clamp(180px,26vw,240px)");
  });

  it("keeps scanner state-driven with clear metrics and safety guidance", () => {
    const scanner = read("src/views/scanner/ScannerView.tsx");
    const overviewModel = read("src/views/overview/overviewModel.ts");
    const scanTaskPanel = read("src/views/overview/ScanTaskPanel.tsx");
    const cancelDialog = read("src/views/overview/ScanCancelDialog.tsx");

    expect(overviewModel).toContain("export type OverviewScanState");
    for (const state of ["scanning", "canceling", "completed", "partial", "canceled", "failed", "first-use"]) {
      expect(overviewModel).toContain(`"${state}"`);
    }
    expect(scanner).toContain("scanState.error");
    expect(scanTaskPanel).toContain('t("overviewScanProcessed")');
    expect(scanTaskPanel).toContain('t("overviewScanElapsed")');
    expect(scanTaskPanel).toContain('t("overviewScanSkipped")');
    expect(scanTaskPanel).toContain('t("overviewScanWarnings")');
    expect(scanTaskPanel).not.toContain("progressbar");
    expect(cancelDialog).toContain("ConfirmDialog");
    expect(scanner).toContain("await cancelScan()");
    expect(scanner).not.toContain("globalThis.confirm");
    expect(scanner).not.toContain("window.confirm");
  });

  it("keeps shell navigation grouped explicitly and page descriptions view-specific", () => {
    const appShell = read("src/components/AppShell.tsx");

    expect(appShell).toContain("function navGroups");
    expect(appShell).toContain('id: "primary"');
    expect(appShell).toContain('id: "advanced"');
    // ADR-0016: the four core jobs lead, and Files is the default destination.
    expect(appShell.indexOf('id: "library"')).toBeLessThan(appShell.indexOf('id: "global-search"'));
    expect(appShell.indexOf('id: "global-search"')).toBeLessThan(appShell.indexOf('id: "organize"'));
    expect(appShell.indexOf('id: "organize"')).toBeLessThan(appShell.indexOf('id: "cleanup"'));
    expect(appShell.indexOf('id: "cleanup"')).toBeLessThan(appShell.indexOf('id: "restore"'));
    expect(appShell.indexOf('id: "restore"')).toBeLessThan(appShell.indexOf('id: "scanner"'));
    expect(appShell).toContain('{ id: "cleanup",');
    expect(appShell).not.toContain('{ id: "preview",');
    expect(appShell).not.toContain("index === 4");
    expect(appShell).toContain('aria-current={view === item.id ? "page" : undefined}');
    expect(appShell).toContain("function viewDescription");
    expect(appShell).toContain('case "cleanup"');
    expect(appShell).toContain('case "automation"');
    expect(appShell).toContain('case "restore"');
    expect(appShell).toContain('case "settings"');
    expect(appShell).toContain("previewActionCount");
  });

  it("auto clears transient success and info toasts without restoring page banners", () => {
    const appShell = read("src/components/AppShell.tsx");

    expect(appShell).toContain("function ToastContainer");
    expect(appShell).toContain("window.setTimeout(clearToast, toast.type === \"success\" ? 2200 : 3200)");
    expect(appShell).toContain("previousViewRef");
    expect(appShell).toContain("if (toast?.type === \"success\") clearToast()");
    expect(appShell).not.toContain("成功：36/36");
  });

  it("keeps titlebar controls draggable-safe and gives mac controls large hit targets", () => {
    const appShell = read("src/components/AppShell.tsx");
    const shellChrome = read("src/components/ShellChrome.tsx");

    expect(appShell).toContain("spotlightButton");
    expect(appShell).toContain("noDrag");
    expect(appShell).toContain("windowsControlButton");
    expect(appShell).toContain("windowsCloseButton");
    expect(appShell).toContain("h-[38px] w-[46px]");
    expect(appShell).toContain("h-6 w-6");
    expect(appShell).toContain("var(--zc-window-close-hover)");
    expect(appShell).not.toContain("overflow-hidden rounded-lg border");
    expect(appShell).toContain("softPanel");
    expect(shellChrome).toContain("titlebarToolButton");
    expect(shellChrome).toContain("aria-label={themeLabel}");
    expect(shellChrome).toContain("title={themeLabel}");
    expect(shellChrome).toContain('t("lightTheme")');
    expect(shellChrome).toContain("[-webkit-app-region:no-drag]");
  });
});
