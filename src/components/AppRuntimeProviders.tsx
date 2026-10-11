import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { tauriApi } from "../api/tauriApi";
import { ChromeProvider, RulesProvider, RuntimeCapabilitiesProvider, SettingsProvider } from "../contexts/AppContexts";
import { useAppChrome } from "../hooks/useAppChrome";
import { defaultScanRootSettingsEqual, enabledScanRootPaths, enabledSearchRootPaths, useAppSettings } from "../hooks/useAppSettings";
import { useFsWatcher } from "../hooks/useFsWatcher";
import { useRulePersistence } from "../hooks/useRulePersistence";
import { useWindowBehavior } from "../hooks/useWindowBehavior";
import { makeTranslator } from "../i18n";
import { useAppStore } from "../store/useAppStore";
import { useBackgroundIndexerStore } from "../store/useBackgroundIndexerStore";
import { useFileLibraryStore } from "../store/useFileLibraryStore";
import { useFileLibraryInspectorStore, useFileLibrarySelectionStore } from "../store/useFileLibraryV2Store";
import { useOperationQueueStore } from "../store/useOperationQueueStore";
import { useRulesStore } from "../store/useRulesStore";
import { useScanManagerStore } from "../store/useScanManagerStore";
import { requestSettingsSection } from "./spotlight/commandRegistry";
import type {
  CloseBehavior,
  FolderNamingLanguage,
  OrganizeRootMode,
  RestoreRetentionDays,
  ScanRootSetting,
  SearchRootSetting,
  SearchScopeMode,
  Rule,
  RuleDraftV2,
  RuntimeCapabilities
} from "../types/domain";
import { useSearchNavigationHandoff } from "../hooks/useSearchNavigationHandoff";
import { projectAcceptedFileLibraryActivation } from "../utils/fileLibraryActivation";
import { localizedStableError, normalizePathLike, readableError } from "../utils/viewHelpers";

export function AppRuntimeProviders({ children }: { children: ReactNode }) {
  const language = useAppStore((state) => state.language);
  const setLanguage = useAppStore((state) => state.setLanguage);
  const theme = useAppStore((state) => state.theme);
  const setTheme = useAppStore((state) => state.setTheme);
  const view = useAppStore((state) => state.view);
  const setView = useAppStore((state) => state.setView);
  const showError = useAppStore((state) => state.showError);
  const enqueueBackgroundIndexRoots = useBackgroundIndexerStore((state) => state.enqueueRoots);
  const rules = useRulesStore((state) => state.rules);
  const upsertRule = useRulesStore((state) => state.upsertRule);
  const removeUserRule = useRulesStore((state) => state.removeUserRule);
  const hydrateUserRulesFromSQLite = useRulesStore((state) => state.hydrateUserRulesFromSQLite);
  const catalogRevision = useRulesStore((state) => state.catalogRevision);
  const setCatalogRevision = useRulesStore((state) => state.setCatalogRevision);
  const [runtimeCapabilities, setRuntimeCapabilities] = useState<RuntimeCapabilities | null>(null);
  const [isLoadingCapabilities, setIsLoadingCapabilities] = useState(true);
  const t = useMemo(() => makeTranslator(language), [language]);

  useEffect(() => {
    let disposed = false;
    setIsLoadingCapabilities(true);
    void tauriApi.getRuntimeCapabilities()
      .then((capabilities) => {
        if (disposed) return;
        setRuntimeCapabilities(capabilities);
      })
      .catch(() => {
        // Capability lookup is defense in depth. Mutating surfaces remain
        // fail-closed on platforms that report an unavailable mutation code.
        if (!disposed) setRuntimeCapabilities(null);
      })
      .finally(() => {
        if (!disposed) setIsLoadingCapabilities(false);
      });
    return () => {
      disposed = true;
    };
  }, []);
  useEffect(() => {
    if (typeof document !== "undefined") {
      document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
    }
  }, [language]);
  const refreshCurrentQuery = useCallback(
    () => useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery),
    []
  );
  const formatSettingsLoadError = useCallback(
    (error: unknown) => `${t("settingsLoadFailed")}：${localizedStableError(error, t)}`,
    [t]
  );
  const formatSettingsSaveError = useCallback(
    (error: unknown) => {
      const detail = localizedStableError(error, t);
      return detail.startsWith(t("settingsSaveFailed"))
        ? detail
        : `${t("settingsSaveFailed")}：${detail}`;
    },
    [t]
  );
  const formatRuleSyncError = useCallback(() => t("ruleSyncFailed"), [t]);
  const reportWindowActionError = useCallback(
    (error: unknown) => showError(`${t("windowActionFailed")}：${readableError(error)}`),
    [showError, t]
  );
  const activateFileLibraryFile = useCallback((fileId: string) => {
    projectAcceptedFileLibraryActivation(fileId, {
      setExplicitSelection: useFileLibrarySelectionStore.getState().setExplicit,
      loadDetail: useFileLibraryInspectorStore.getState().loadDetail
    });
  }, []);

  const appSettingsState = useAppSettings({
    isDatabaseReady: true,
    onError: showError,
    formatLoadError: formatSettingsLoadError,
    formatSaveError: formatSettingsSaveError
  });
  const {
    settings: appSettings,
    persistedSettings,
    isLoadingSettings,
    updateSettings,
    updateSettingsWithResult
  } = appSettingsState;
  const appChrome = useAppChrome({
    theme,
    setTheme,
    setLanguage,
    searchHotkey: appSettings.searchHotkey
  });
  const {
    commandInputRef,
    isCommandOpen,
    setIsCommandOpen,
    platform,
    isWindows,
    effectiveTheme,
    hotkeyLabel,
    isSearchMode
  } = appChrome;
  useRulePersistence({
    enabled: !isSearchMode,
    isDatabaseReady: true,
    rules,
    hydrateUserRulesFromSQLite,
    onCatalogRevision: setCatalogRevision,
    onError: showError,
    formatSyncError: formatRuleSyncError
  });
  useFsWatcher({ onRefreshData: refreshCurrentQuery, onError: showError, rules, enabled: !isSearchMode });

  useEffect(() => {
    if (isSearchMode || isLoadingSettings) return;
    useScanManagerStore.getState().setDefaultScanRoots(persistedSettings.defaultScanFolders);
    const library = useFileLibraryStore.getState();
    if (library.adoptConfiguredRootsIfScopeEmpty(persistedSettings.defaultScanFolders)) {
      void library.refresh(useAppStore.getState().searchQuery);
    }
  }, [isLoadingSettings, isSearchMode, persistedSettings.defaultScanFolders]);

  const backgroundIndexRoots = useMemo(
    () => [
      ...enabledScanRootPaths(persistedSettings.defaultScanFolders),
      ...enabledSearchRootPaths(persistedSettings.customSearchRoots)
    ],
    [persistedSettings.defaultScanFolders, persistedSettings.customSearchRoots]
  );
  const backgroundIndexRootSignature = useMemo(
    () => backgroundIndexRoots.map(backgroundIndexRootKey).sort().join("\n"),
    [backgroundIndexRoots]
  );

  useEffect(() => {
    if (isSearchMode || isLoadingSettings) return;
    if (persistedSettings.backgroundIndexOnStartup === false) return;
    enqueueBackgroundIndexRoots(backgroundIndexRoots);
  }, [
    backgroundIndexRootSignature,
    enqueueBackgroundIndexRoots,
    isLoadingSettings,
    isSearchMode,
    persistedSettings.backgroundIndexOnStartup
  ]);

  useSearchNavigationHandoff(isSearchMode, setView, activateFileLibraryFile, showError);


  useEffect(() => {
    if (isSearchMode) return;

    let disposed = false;
    let unlisten: (() => void) | undefined;

    void tauriApi.onGlobalHotkeyRegistrationFailed((payload) => {
      useAppStore.getState().setGlobalHotkeyError(payload.message);
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch((error) => {
      if (!disposed) showError(readableError(error));
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [isSearchMode, showError]);

  const setCloseBehavior = useCallback(
    async (next: CloseBehavior) => {
      const savedSettings = await updateSettings({ closeBehavior: next });
      return savedSettings.closeBehavior === next;
    },
    [updateSettings]
  );
  const setFolderNamingLanguage = useCallback(
    async (next: FolderNamingLanguage) => {
      const savedSettings = await updateSettings({ folderNamingLanguage: next });
      return savedSettings.folderNamingLanguage === next;
    },
    [updateSettings]
  );
  const setDefaultScanFolders = useCallback(
    async (next: ScanRootSetting[]) => {
      const result = await updateSettingsWithResult({ defaultScanFolders: next });
      const rootsMatch = defaultScanRootSettingsEqual(result.settings.defaultScanFolders, next);
      if (result.persisted && !rootsMatch) {
        showError(`${t("settingsSaveFailed")} (${t("settingsFailureCode")}: unknown_failure)`);
      }
      return result.persisted && rootsMatch;
    },
    [showError, t, updateSettingsWithResult]
  );
  const setRestoreRetentionDays = useCallback(
    async (next: RestoreRetentionDays) => {
      const savedSettings = await updateSettings({ restoreRetentionDays: next });
      return savedSettings.restoreRetentionDays === next;
    },
    [updateSettings]
  );
  const setLaunchAtLogin = useCallback(
    async (next: boolean) => {
      const savedSettings = await updateSettings({ launchAtLogin: next });
      return savedSettings.launchAtLogin === next;
    },
    [updateSettings]
  );
  const setBackgroundIndexOnStartup = useCallback(
    async (next: boolean) => {
      const savedSettings = await updateSettings({ backgroundIndexOnStartup: next });
      return savedSettings.backgroundIndexOnStartup === next;
    },
    [updateSettings]
  );
  const setSearchHotkey = useCallback(
    async (next: string) => {
      const previous = appSettings.searchHotkey;
      try {
        const status = await tauriApi.registerGlobalSearchHotkey(next);
        useAppStore.getState().setGlobalHotkeyError(status.error ?? "");
        if (
          !status.registered
          || status.effectiveAccelerator !== next
          || status.error
        ) {
          if (status.error) showError(status.error);
          return false;
        }
      } catch (error) {
        const message = readableError(error);
        useAppStore.getState().setGlobalHotkeyError(message);
        showError(message);
        return false;
      }

      const savedSettings = await updateSettings({ searchHotkey: next });
      if (savedSettings.searchHotkey === next) return true;

      try {
        const rollbackStatus = await tauriApi.registerGlobalSearchHotkey(previous);
        useAppStore.getState().setGlobalHotkeyError(rollbackStatus.error ?? "");
        if (!rollbackStatus.registered) {
          const message = rollbackStatus.error ?? "Failed to restore the previous global hotkey.";
          useAppStore.getState().setGlobalHotkeyError(message);
          showError(message);
        }
      } catch (error) {
        const message = `Failed to restore the previous global hotkey: ${readableError(error)}`;
        useAppStore.getState().setGlobalHotkeyError(message);
        showError(message);
      }
      return false;
    },
    [appSettings.searchHotkey, showError, updateSettings]
  );
  const setSearchScopeMode = useCallback(
    async (next: SearchScopeMode) => {
      const savedSettings = await updateSettings({ searchScopeMode: next });
      return savedSettings.searchScopeMode === next;
    },
    [updateSettings]
  );
  const setCustomSearchRoots = useCallback(
    async (next: SearchRootSetting[]) => {
      const savedSettings = await updateSettings({ customSearchRoots: next });
      return arraysEqual(savedSettings.customSearchRoots, next);
    },
    [updateSettings]
  );
  const setOrganizeRootMode = useCallback(
    async (next: OrganizeRootMode) => {
      const savedSettings = await updateSettings({ organizeRootMode: next });
      return savedSettings.organizeRootMode === next;
    },
    [updateSettings]
  );
  const setOrganizeRootPath = useCallback(
    async (next?: string) => {
      const normalized = normalizeOptionalPath(next);
      const savedSettings = await updateSettings({ organizeRootPath: normalized });
      return normalizeOptionalPath(savedSettings.organizeRootPath) === normalized;
    },
    [updateSettings]
  );
  const windowBehavior = useWindowBehavior({
    closeBehavior: appSettings.closeBehavior,
    setCloseBehavior,
    lastView: view,
    onError: reportWindowActionError
  });
  const {
    closeBehavior,
    isCloseChoiceOpen,
    onCancelCloseChoice,
    handleWindowAction,
    requestClose,
    resolveCloseChoice
  } = windowBehavior;

  const saveRule = useCallback(async (rule: Rule) => {
    const existing = rules.find((current) => current.id === rule.id);
    const draft = ruleDraftV2(rule);
    const result = existing
      ? await tauriApi.updateUserRuleV2(
        existing.id,
        requireRuleRevision(existing),
        catalogRevision,
        draft
      )
      : await tauriApi.createUserRuleV2(draft, catalogRevision);
    upsertRule(result.rule, result.catalogRevision);
  }, [catalogRevision, rules, upsertRule]);
  const toggleRuleEnabled = useCallback(async (rule: Rule, enabled: boolean) => {
    const result = await tauriApi.setUserRuleEnabledV2(
      rule.id,
      requireRuleRevision(rule),
      catalogRevision,
      enabled
    );
    upsertRule(result.rule, result.catalogRevision);
  }, [catalogRevision, upsertRule]);
  const deleteRule = useCallback(async (rule: Rule) => {
    if (rule.source !== "user") {
      return false;
    }
    const state = await tauriApi.deleteUserRuleV2(
      rule.id,
      requireRuleRevision(rule),
      catalogRevision,
      true
    );
    removeUserRule(rule.id, state.revision);
    return true;
  }, [catalogRevision, removeUserRule]);

  const settingsContextValue = useMemo(() => ({
    settings: appSettings,
    isLoadingSettings,
    updateSettings,
    setFolderNamingLanguage,
    setDefaultScanFolders,
    setRestoreRetentionDays,
    setLaunchAtLogin,
    setBackgroundIndexOnStartup,
    setSearchHotkey,
    setSearchScopeMode,
    setCustomSearchRoots,
    setOrganizeRootMode,
    setOrganizeRootPath
  }), [
    appSettings,
    isLoadingSettings,
    updateSettings,
    setFolderNamingLanguage,
    setDefaultScanFolders,
    setRestoreRetentionDays,
    setLaunchAtLogin,
    setBackgroundIndexOnStartup,
    setSearchHotkey,
    setSearchScopeMode,
    setCustomSearchRoots,
    setOrganizeRootMode,
    setOrganizeRootPath
  ]);
  const rulesContextValue = useMemo(() => ({
    rules,
    saveRule,
    toggleRuleEnabled,
    deleteRule
  }), [deleteRule, rules, saveRule, toggleRuleEnabled]);
  const chromeContextValue = useMemo(() => ({
    commandInputRef,
    isCommandOpen,
    setIsCommandOpen,
    platform,
    isWindows,
    effectiveTheme,
    hotkeyLabel,
    isSearchMode,
    closeBehavior,
    setCloseBehavior,
    isCloseChoiceOpen,
    onCancelCloseChoice,
    handleWindowAction,
    requestClose,
    resolveCloseChoice,
    language,
    setLanguage,
    theme,
    setTheme,
    view,
    setView,
    onError: showError,
    t
  }), [
    commandInputRef,
    isCommandOpen,
    setIsCommandOpen,
    platform,
    isWindows,
    effectiveTheme,
    hotkeyLabel,
    isSearchMode,
    closeBehavior,
    setCloseBehavior,
    isCloseChoiceOpen,
    onCancelCloseChoice,
    handleWindowAction,
    requestClose,
    resolveCloseChoice,
    language,
    setLanguage,
    theme,
    setTheme,
    view,
    setView,
    showError,
    t
  ]);

  const runtimeCapabilitiesContextValue = useMemo(() => ({
    capabilities: runtimeCapabilities,
    isLoadingCapabilities
  }), [isLoadingCapabilities, runtimeCapabilities]);

  return (
    <ChromeProvider value={chromeContextValue}>
      <RuntimeCapabilitiesProvider value={runtimeCapabilitiesContextValue}>
        <StoreRuntimeBootstrapper enabled={!isSearchMode} />
        <SettingsProvider value={settingsContextValue}>
          <RulesProvider value={rulesContextValue}>{children}</RulesProvider>
        </SettingsProvider>
      </RuntimeCapabilitiesProvider>
    </ChromeProvider>
  );
}

function StoreRuntimeBootstrapper({ enabled }: { enabled: boolean }) {
  const initializeScanListeners = useScanManagerStore((state) => state.initializeScanListeners);
  const initializeOperationQueue = useOperationQueueStore((state) => state.initializeOperationQueue);

  useEffect(() => {
    if (!enabled) return;

    void useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery);
    void initializeScanListeners();
    void initializeOperationQueue();
  }, [enabled, initializeOperationQueue, initializeScanListeners]);

  return null;
}

function arraysEqual<T>(left: readonly T[], right: readonly T[]) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function backgroundIndexRootKey(path: string) {
  return normalizePathLike(path.trim());
}

function normalizeOptionalPath(path?: string | null) {
  const normalized = path?.trim().replace(/\\+/g, "/").replace(/\/+$/g, "");
  return normalized || undefined;
}

function requireRuleRevision(rule: Rule) {
  if (!Number.isInteger(rule.revision) || (rule.revision ?? 0) < 1) {
    throw new Error("rule_revision_missing");
  }
  return rule.revision as number;
}

function ruleDraftV2(rule: Rule): RuleDraftV2 {
  return {
    name: rule.name,
    priority: rule.priority,
    weight: rule.weight,
    rootOperator: rule.root_operator === "OR" ? "OR" : "AND",
    groups: rule.groups.map((group) => ({
      operator: group.operator === "OR" ? "OR" : "AND",
      conditions: group.conditions.map((condition) => ({
        field: condition.field as Exclude<typeof condition.field, "unknown">,
        operator: condition.operator as Exclude<typeof condition.operator, "unknown">,
        value: condition.value
      }))
    })),
    action: {
      purpose: rule.action.purpose,
      lifecycle: rule.action.lifecycle,
      context: rule.action.context,
      riskLevel: rule.action.risk_level,
      suggestedAction: rule.action.suggested_action,
      targetTemplate: rule.action.target_template,
      renameTemplate: rule.action.rename_template
    }
  };
}
