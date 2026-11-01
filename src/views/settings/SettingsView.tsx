import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { tauriApi, type GlobalHotkeyStatus } from "../../api/tauriApi";
import { useCommandContext, useI18nContext, useNavigationContext, useSettingsContext, useThemeContext, useWindowContext } from "../../contexts/AppContexts";
import {
  removeSearchRoot,
  removeDefaultScanRoot,
  toggleSearchRoot,
  toggleDefaultScanRoot,
  upsertSearchRoot,
  upsertDefaultScanRoot
} from "../../hooks/useAppSettings";
import { useScanManagerStore } from "../../store/useScanManagerStore";
import { useAppStore } from "../../store/useAppStore";
import { useBackgroundIndexerStore } from "../../store/useBackgroundIndexerStore";
import { useAIProcessingModeStore } from "../../store/useAIProcessingModeStore";
import { useWatcherStatusStore } from "../../store/useWatcherStatusStore";
import {
  explicitSingleSelectionId,
  useFileLibraryInspectorStore,
  useFileLibrarySelectionStore
} from "../../store/useFileLibraryV2Store";
import type {
  AIConnectionTestResult,
  AICustomProviderProfile,
  AIDebugClassificationResult,
  AIModelInfo,
  AIRequestTrace,
  AIProviderPreset,
  AIProviderPresetId,
  AISettings,
  AiManagementStatus,
  CloseBehavior,
  FolderNamingLanguage,
  ManagedScope,
  OrganizeRootMode,
  RestoreRetentionDays,
  RuntimeCapabilities,
  ScanRootSetting,
  SearchRootSetting,
  SearchScopeMode
} from "../../types/domain";
import type { Translator } from "../../types/ui";
import {
  aiSettingsEqual,
  applyAIClassificationPreset,
  resolveAIClassificationPreset,
  type AIClassificationPresetId
} from "./aiSettingsModel";
import { acceleratorFromKeyboardEvent, formatHotkeyLabel, isValidSearchHotkey } from "../../utils/hotkeys";
import { compactPath, localizedStableError, normalizePathLike, readableError } from "../../utils/viewHelpers";
import { buttonSecondary, cn } from "../../utils/tw";
import {
  ConfirmDialog,
  quietText
} from "../shared/ui";
import {
  SettingsControlGroup,
  SettingsDisclosure,
  SettingsEmptyState,
  SettingsInlineMessage,
  SettingsLayout,
  SettingsRow,
  SettingsSearch,
  SettingsSegmentedControl,
  SettingsSelect,
  SettingsSwitch,
  SettingsTextField,
  settingsField
} from "./components/SettingsPrimitives";
import { SettingsSecretField } from "./components/SettingsSecretField";
import { AboutSettingsSection } from "./sections/AboutSettingsSection";
import { AISettingsSection } from "./sections/AISettingsSection";
import { AI_SETTINGS_MODE_REQUEST_KEY } from "./settingsNavigation";
import { AutomationSettingsSection } from "./sections/AutomationSettingsSection";
import { FileSourcesSettingsSection } from "./sections/FileSourcesSettingsSection";
import { GeneralSettingsSection } from "./sections/GeneralSettingsSection";
import { GlobalIndexSettingsSection } from "./sections/GlobalIndexSettingsSection";
import { GlobalSearchSettingsSection } from "./sections/GlobalSearchSettingsSection";
import { ManagedLibrarySettingsSection } from "./sections/ManagedLibrarySettingsSection";
import { PrivacyContentSettingsSection } from "./sections/PrivacyContentSettingsSection";
import { PlatformDiagnosticsSettingsSection } from "./sections/PlatformDiagnosticsSettingsSection";
import { DeveloperDiagnosticsSection } from "./sections/DeveloperDiagnosticsSection";
import type { FolderDeleteConfirmState } from "./sections/settingsSectionTypes";
import { SETTINGS_SECTION_IDS, useSettingsNavigationController } from "./controllers/useSettingsNavigationController";
import { useSettingsGlobalIndexController } from "./controllers/useSettingsGlobalIndexController";
import { projectSettingsSelectedFile, settingsSelectedFileDetailRequestId } from "./settingsSelectedFile";

type StatusTone = "success" | "warning";
type AIUserMode = "off" | "local" | "cloud";
const DEVELOPER_MODE_STORAGE_KEY = "zc-developer-mode";
const AI_CLASSIFICATION_PRESET_IDS = ["fast", "standard", "detailed", "custom"] as const;
const AI_CLASSIFICATION_LABEL_KEYS: Record<typeof AI_CLASSIFICATION_PRESET_IDS[number], Parameters<Translator>[0]> = {
  fast: "aiPresetFast",
  standard: "aiPresetStandard",
  detailed: "aiPresetDetailed",
  custom: "aiPresetCustom"
};
function aiUserMode(settings: Pick<AISettings, "enabled" | "provider">): AIUserMode {
  if (!settings.enabled) return "off";
  return settings.provider === "ollama" ? "local" : "cloud";
}

function userProviderValue(settings: AISettings): AIProviderPresetId | "openai_compatible" | "custom" {
  if (settings.provider === "ollama") return "ollama";
  if (settings.preset === "deepseek") return "deepseek";
  if (settings.preset === "custom_openai_compatible") return "custom";
  return "openai_compatible";
}

function applyProviderPreset(settings: AISettings, preset: AIProviderPreset): AISettings {
  return {
    ...settings,
    enabled: settings.enabled,
    provider: preset.providerKind,
    preset: preset.id,
    baseUrl: preset.defaultBaseUrl,
    chatPath: preset.defaultChatPath,
    modelsPath: preset.modelsPath ?? null,
    model: preset.defaultModel,
    apiKey: settings.apiKey
  };
}

function readDeveloperMode() {
  try {
    return window.localStorage.getItem(DEVELOPER_MODE_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function globalIndexStatusText(status: string, t: Translator) {
  const statusKeys: Record<string, Parameters<Translator>[0]> = {
    discovered: "globalIndexStatusDiscovered",
    authorization_required: "globalIndexStatusAuthorizationRequired",
    indexing: "globalIndexStatusIndexing",
    syncing: "globalIndexStatusSyncing",
    ready: "globalIndexStatusReady",
    partial: "globalIndexStatusPartial",
    paused: "globalIndexStatusPaused",
    rebuild_required: "globalIndexStatusRebuildRequired",
    permission_required: "globalIndexStatusPermissionRequired",
    watcher_unavailable: "globalIndexStatusWatcherUnavailable",
    database_busy: "globalIndexStatusDatabaseBusy",
    service_busy: "globalIndexStatusServiceBusy",
    spotlight_unavailable: "globalIndexStatusSpotlightUnavailable",
    spotlight_not_indexed: "globalIndexStatusSpotlightNotIndexed",
    spotlight_external_not_indexed: "globalIndexStatusSpotlightExternalNotIndexed",
    fsevents_unavailable: "globalIndexStatusFseventsUnavailable",
    unavailable: "globalIndexStatusUnavailable",
    error: "globalIndexStatusError"
  };
  const key = statusKeys[status] ?? "globalIndexStatusUnknown";
  return t(key);
}

function globalIndexProviderStatusText(status: string | null | undefined, t: Translator) {
  if (!status) return null;
  if (status.includes("service_unavailable")) return t("globalIndexServiceUnavailable");
  if (status.startsWith("windows_index_service:")) return t("globalIndexServiceRunning");
  return t("globalIndexProviderDirectFallback");
}

function globalIndexErrorText(error: string | null | undefined, t: Translator) {
  if (!error) return null;
  const message = error.replace(/^provider error:\s*/i, "");
  if (message === "global_index_database_busy") return t("globalIndexErrorDatabaseBusy");
  if (message === "global_index_service_busy") return t("globalIndexErrorServiceBusy");
  if (message === "global_index_permission_required") return t("globalIndexErrorPermissionRequired");
  if (message.startsWith("macos_spotlight_no_indexed_local_results")) {
    return t("globalIndexErrorSpotlightNoResults");
  }
  if (message.startsWith("macos_spotlight_full_disk_access_required")) {
    return t("globalIndexErrorSpotlightFullDiskAccess");
  }
  if (message.startsWith("macos_spotlight_external_volume_not_indexed")) {
    return t("globalIndexErrorSpotlightExternalNotIndexed");
  }
  if (message.startsWith("macos_spotlight_partial_results_permission_required") || message.startsWith("macos_spotlight_protected_directories")) {
    return t("globalIndexErrorSpotlightProtected");
  }
  if (message.startsWith("macos_spotlight_incomplete_results")) {
    return t("globalIndexErrorSpotlightIncomplete");
  }
  if (message.startsWith("macos_spotlight")) {
    return t("globalIndexErrorSpotlightUnavailable");
  }
  if (message.startsWith("macos_fsevents")) {
    return t("globalIndexErrorRealtimeUnavailable");
  }
  if (message === "windows_global_index_unstable_volume_identity") {
    return t("globalIndexErrorWindowsStableIdentity");
  }
  if (message === "windows_global_index_unsupported_virtual_volume") {
    return t("globalIndexErrorWindowsUsnJournalUnavailable");
  }
  if (message.startsWith("windows_global_index_unsupported_filesystem:")) {
    return t("globalIndexErrorWindowsUnsupportedFilesystem");
  }
  if (message.startsWith("windows_global_index_unsupported_drive_kind:")) {
    return t("globalIndexErrorWindowsUnsupportedDriveKind");
  }
  if (message === "windows_global_index_native_provider_unsupported" || message === "global_index_source_not_authorizable") {
    return t("globalIndexErrorWindowsVolumeUnsupported");
  }
  if (message.startsWith("global_index_source_unavailable")) {
    return t("globalIndexErrorSourceUnavailable");
  }
  return t("globalIndexErrorGeneric");
}

function managedScopePolicyText(policySummary: string | undefined, t: Translator) {
  const policyKeys: Record<string, Parameters<Translator>[0]> = {
    managed_scope_only_cloud_enabled: "managedScopeOnlyCloudEnabled",
    managed_scope_only_cloud_disabled: "managedScopeOnlyCloudDisabled"
  };
  return t(policyKeys[policySummary ?? ""] ?? "managedScopePolicySummary");
}

export function SettingsView() {
  const { language, setLanguage, t } = useI18nContext();
  const { setView } = useNavigationContext();
  const { platform } = useCommandContext();
  const { theme, setTheme } = useThemeContext();
  const { closeBehavior, setCloseBehavior } = useWindowContext();
  const {
    settings: {
      folderNamingLanguage,
      defaultScanFolders,
      restoreRetentionDays,
      launchAtLogin,
      backgroundIndexOnStartup,
      searchHotkey,
      searchScopeMode,
      customSearchRoots,
      organizeRootMode,
      organizeRootPath,
      useLegacyBuiltinClassificationRules,
      useLearnedRulesAsAutoRules
    },
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
  } = useSettingsContext();
  const scanPath = useScanManagerStore((state) => state.scanPath);
  const pendingBackgroundRoots = useBackgroundIndexerStore((state) => state.pendingRoots);
  const currentBackgroundRoot = useBackgroundIndexerStore((state) => state.currentRoot);
  const isBackgroundIndexing = useBackgroundIndexerStore((state) => state.isBackgroundIndexing);
  const failedBackgroundRoots = useBackgroundIndexerStore((state) => state.failedRoots);
  const completedBackgroundRoots = useBackgroundIndexerStore((state) => state.completedRoots);
  const enqueueBackgroundIndexRoot = useBackgroundIndexerStore((state) => state.enqueueRoot);
  const watcherRootStatuses = useWatcherStatusStore((state) => state.roots);
  const librarySelection = useFileLibrarySelectionStore((state) => state.selection);
  const selectedLibraryFileId = explicitSingleSelectionId(librarySelection);
  const inspectorSelectedId = useFileLibraryInspectorStore((state) => state.selectedId);
  const inspectorDetail = useFileLibraryInspectorStore((state) => state.detail);
  const inspectorIsLoading = useFileLibraryInspectorStore((state) => state.isLoading);
  const inspectorError = useFileLibraryInspectorStore((state) => state.error);
  const loadLibraryDetail = useFileLibraryInspectorStore((state) => state.loadDetail);
  const selectedLibraryFile = projectSettingsSelectedFile(selectedLibraryFileId, {
    selectedId: inspectorSelectedId,
    detail: inspectorDetail
  });
  const globalHotkeyError = useAppStore((state) => state.globalHotkeyError);
  const setGlobalHotkeyError = useAppStore((state) => state.setGlobalHotkeyError);
  const density = useAppStore((state) => state.density) ?? "default";
  const setDensity = useAppStore((state) => state.setDensity);
  const hotkey = formatHotkeyLabel(searchHotkey, platform);
  const [settingsStatus, setSettingsStatus] = useState("");
  const [settingsStatusTone, setSettingsStatusTone] = useState<StatusTone>("success");
  const [isRecordingHotkey, setIsRecordingHotkey] = useState(false);
  const [recordingHotkeyPreview, setRecordingHotkeyPreview] = useState("");
  const [globalHotkeyStatus, setGlobalHotkeyStatus] = useState<GlobalHotkeyStatus | null>(null);
  const [folderDeleteConfirm, setFolderDeleteConfirm] = useState<FolderDeleteConfirmState | null>(null);
  const [isDeletingFolderConfig, setIsDeletingFolderConfig] = useState(false);
  const [aiSettings, setAiSettings] = useState<AISettings | null>(null);
  const [persistedAISettings, setPersistedAISettings] = useState<AISettings | null>(null);
  const runtimeAISettings = useAIProcessingModeStore((state) => state.settings);
  const runtimeAIStatus = useAIProcessingModeStore((state) => state.status);
  const publishAIProcessingMode = useAIProcessingModeStore((state) => state.publish);
  const [aiPresets, setAiPresets] = useState<AIProviderPreset[]>([]);
  const [runtimeCapabilities, setRuntimeCapabilities] = useState<RuntimeCapabilities | null>(null);
  const [aiModels, setAiModels] = useState<AIModelInfo[]>([]);
  const [isDiscoveringAIModels, setIsDiscoveringAIModels] = useState(false);
  const [aiTraces, setAiTraces] = useState<AIRequestTrace[]>([]);
  const [isLoadingAITraces, setIsLoadingAITraces] = useState(false);
  const [isLoadingAISettings, setIsLoadingAISettings] = useState(false);
  const [isSavingAISettings, setIsSavingAISettings] = useState(false);
  const [aiSettingsSaveError, setAiSettingsSaveError] = useState(false);
  const [isTestingAIConnection, setIsTestingAIConnection] = useState(false);
  const [aiConnectionStatus, setAiConnectionStatus] = useState<{ tone: StatusTone; message: string; role?: "status" | "alert" } | null>(null);
  const [aiDebugTarget, setAiDebugTarget] = useState("");
  const [isDebuggingAI, setIsDebuggingAI] = useState(false);
  const [aiDebugStatus, setAiDebugStatus] = useState<{ tone: StatusTone; message: string; role?: "status" | "alert" } | null>(null);
  const [aiDebugResult, setAiDebugResult] = useState<AIDebugClassificationResult | null>(null);
  const [developerMode, setDeveloperMode] = useState(readDeveloperMode);
  const [aiAdvancedOpen, setAiAdvancedOpen] = useState(false);
  const [secretRevealResetVersion, setSecretRevealResetVersion] = useState(0);
  const hotkeyCaptureRef = useRef<HTMLDivElement | null>(null);
  const aiSaveRequestRef = useRef(0);
  const translatorRef = useRef(t);
  translatorRef.current = t;

  const { activeSettingsSection, focusSettingsSection, settingsScrollRef } = useSettingsNavigationController();
  const {
    globalIndexStatus,
    globalIndexSources,
    globalIndexLoadState,
    globalIndexLoadError,
    managedScopes,
    aiManagementStatus,
    managedScopePath,
    setManagedScopePath,
    isLoadingGlobalIndex,
    isUpdatingGlobalIndex,
    refreshGlobalIndexData,
    runGlobalIndexAction,
    addManagedScopeFromSettings,
    updateManagedScope,
    removeManagedScope
  } = useSettingsGlobalIndexController({ t, showStatus });

  const aiSettingsDirty = Boolean(aiSettings && persistedAISettings && !aiSettingsEqual(aiSettings, persistedAISettings));
  const activeAIClassificationPreset = aiSettings ? resolveAIClassificationPreset(aiSettings) : "custom";
  const aiDependentControlsDisabled = !aiSettings?.enabled;
  const runtimeAIUserMode = runtimeAISettings ? aiUserMode(runtimeAISettings) : persistedAISettings ? aiUserMode(persistedAISettings) : "off";
  const draftAIUserMode = aiSettings ? aiUserMode(aiSettings) : "off";

  useEffect(() => {
    const detailRequestId = settingsSelectedFileDetailRequestId(selectedLibraryFileId, {
      selectedId: inspectorSelectedId,
      detail: inspectorDetail,
      isLoading: inspectorIsLoading,
      error: inspectorError
    });
    if (!detailRequestId) return;
    void loadLibraryDetail(detailRequestId);
  }, [inspectorDetail?.id, inspectorError, inspectorIsLoading, inspectorSelectedId, loadLibraryDetail, selectedLibraryFileId]);

  const settingsSections = [
    { id: "settings-general", label: t("settingsGeneral") },
    { id: "settings-files-scan", label: t("settingsFilesScan") },
    { id: "settings-search", label: t("settingsSearch") },
    { id: "settings-global-index", label: t("globalIndexSettings") },
    { id: "settings-platform-diagnostics", label: t("platformDiagnosticsNav") },
    { id: "settings-managed-scopes", label: t("managedScopesTitle") },
    { id: "settings-automation", label: t("settingsSmartOrganize") },
    { id: "settings-ai", label: t("settingsAIPrivacy") },
    { id: "settings-privacy", label: t("settingsDiagnostics") },
    { id: "settings-about", label: t("settingsAbout") }
  ];

  function setDeveloperModePreference(next: boolean) {
    setDeveloperMode(next);
    try {
      window.localStorage.setItem(DEVELOPER_MODE_STORAGE_KEY, String(next));
    } catch {
      // Optional local preference; advanced controls remain fail-closed when storage is unavailable.
    }
  }

  useEffect(() => {
    let disposed = false;
    void tauriApi.getGlobalHotkeyStatus().then((status) => {
      if (!disposed) {
        setGlobalHotkeyStatus(status);
        setGlobalHotkeyError(status?.error ?? "");
      }
    }).catch(() => {
      // Browser previews and test shells may not expose desktop-only commands.
    });
    return () => {
      disposed = true;
    };
  }, [setGlobalHotkeyError]);

  useEffect(() => {
    if (!settingsStatus) return;
    const timer = setTimeout(() => setSettingsStatus(""), settingsStatusTone === "success" ? 2400 : 5200);
    return () => clearTimeout(timer);
  }, [settingsStatus, settingsStatusTone]);

  useEffect(() => {
    if (!isRecordingHotkey) return;
    hotkeyCaptureRef.current?.focus();

    function handleWindowKeyDown(event: globalThis.KeyboardEvent) {
      recordHotkeyFromEvent(event);
    }

    window.addEventListener("keydown", handleWindowKeyDown);
    return () => window.removeEventListener("keydown", handleWindowKeyDown);
  }, [isRecordingHotkey, hotkey, platform]);

  useEffect(() => {
    let disposed = false;
    setIsLoadingAISettings(true);
    void Promise.all([tauriApi.listAIProviderPresets(), tauriApi.getAISettings(), tauriApi.getRuntimeCapabilities()])
      .then(([presets, settings, capabilities]) => {
        if (disposed) return;
        setAiPresets(presets);
        setRuntimeCapabilities(capabilities);
        setAiSettings(settings);
        setPersistedAISettings(settings);
        setAiSettingsSaveError(false);
        publishAIProcessingMode({ enabled: settings.enabled, provider: settings.provider });
      })
      .catch((error) => {
        if (!disposed) {
          setAiConnectionStatus({ tone: "warning", role: "alert", message: `${t("aiSettingsLoadFailed")}：${localizedStableError(error, t)}` });
        }
      })
      .finally(() => {
        if (!disposed) setIsLoadingAISettings(false);
      });
    return () => {
      disposed = true;
      aiSaveRequestRef.current += 1;
    };
  }, [publishAIProcessingMode]);

  function showStatus(message: string, tone: StatusTone = "success") {
    setSettingsStatus(message);
    setSettingsStatusTone(tone);
  }

  async function pickFolder(title: string) {
    try {
      const selectedPath = await open({ directory: true, multiple: false, title });
      const path = Array.isArray(selectedPath) ? selectedPath[0] : selectedPath;
      return path?.trim() || null;
    } catch (error) {
      showStatus(`${t("folderPickerFailed")}：${readableError(error)}`, "warning");
      return null;
    }
  }

  async function updateCloseBehavior(next: CloseBehavior) {
    const saved = await setCloseBehavior(next);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function updateFolderNamingLanguage(next: FolderNamingLanguage) {
    const saved = await setFolderNamingLanguage(next);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function updateLaunchAtLogin(next: boolean) {
    const saved = await setLaunchAtLogin(next);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function updateBackgroundIndexOnStartup(next: boolean) {
    const saved = await setBackgroundIndexOnStartup(next);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function addDefaultScanFolder() {
    const path = await pickFolder(t("folderPickerTitle"));
    if (!path) return;

    const saved = await setDefaultScanFolders(upsertDefaultScanRoot(defaultScanFolders, path));
    if (saved) {
      showStatus(`${t("settingsSavedInline")} · ${t("defaultScanFoldersRestartHint")}`);
    }
  }

  async function setScanRootEnabled(root: ScanRootSetting, enabled: boolean) {
    const saved = await setDefaultScanFolders(toggleDefaultScanRoot(defaultScanFolders, root.id, enabled));
    if (saved) {
      showStatus(`${t("settingsSavedInline")} · ${t("defaultScanFoldersRestartHint")}`);
    }
  }

  async function deleteScanRoot(root: ScanRootSetting) {
    const saved = await setDefaultScanFolders(removeDefaultScanRoot(defaultScanFolders, root.id));
    if (saved) {
      showStatus(`${t("settingsSavedInline")} · ${t("defaultScanFoldersRestartHint")}`);
    }
    return saved;
  }

  async function updateSearchScopeMode(next: SearchScopeMode) {
    const saved = await setSearchScopeMode(next);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function updateOrganizeRootMode(next: OrganizeRootMode) {
    const saved = await setOrganizeRootMode(next);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function chooseOrganizeRootPath() {
    const path = await pickFolder(t("organizeRootPickerTitle"));
    if (!path) return;

    const saved = await setOrganizeRootPath(path);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function updateOrganizeRootPath(next: string) {
    const saved = await setOrganizeRootPath(next);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function addCustomSearchRoot() {
    const path = await pickFolder(t("folderPickerTitle"));
    if (!path) return;

    const saved = await setCustomSearchRoots(upsertSearchRoot(customSearchRoots, path));
    if (saved) {
      enqueueBackgroundIndexRoot(path);
      showStatus(`${t("settingsSavedInline")} · ${t("backgroundIndexQueued")}`);
    }
  }

  async function setSearchRootEnabled(root: SearchRootSetting, enabled: boolean) {
    const saved = await setCustomSearchRoots(toggleSearchRoot(customSearchRoots, root.id, enabled));
    if (saved) {
      if (enabled) enqueueBackgroundIndexRoot(root.path, { force: true });
      showStatus(enabled ? `${t("settingsSavedInline")} · ${t("backgroundIndexQueued")}` : t("settingsSavedInline"));
    }
  }

  async function deleteSearchRoot(root: SearchRootSetting) {
    const saved = await setCustomSearchRoots(removeSearchRoot(customSearchRoots, root.id));
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
    return saved;
  }

  async function confirmFolderDelete() {
    if (!folderDeleteConfirm || isDeletingFolderConfig) return;
    setIsDeletingFolderConfig(true);
    try {
      const saved = folderDeleteConfirm.kind === "scan"
        ? await deleteScanRoot(folderDeleteConfirm.root)
        : await deleteSearchRoot(folderDeleteConfirm.root);
      if (saved) setFolderDeleteConfirm(null);
    } finally {
      setIsDeletingFolderConfig(false);
    }
  }

  async function scanRootNow(root: ScanRootSetting) {
    await scanPath(root.path);
  }

  function indexSearchRootNow(root: SearchRootSetting) {
    enqueueBackgroundIndexRoot(root.path, { force: true });
    showStatus(t("backgroundIndexQueued"));
  }

  function backgroundRootState(root: SearchRootSetting) {
    const normalized = normalizeSettingsRoot(root.path);
    if (currentBackgroundRoot && normalizeSettingsRoot(currentBackgroundRoot) === normalized) return "indexing";
    if (pendingBackgroundRoots.some((path) => normalizeSettingsRoot(path) === normalized)) return "queued";
    if (completedBackgroundRoots.some((path) => normalizeSettingsRoot(path) === normalized)) return "completed";
    return "idle";
  }

  async function updateRestoreRetentionDays(next: RestoreRetentionDays) {
    const saved = await setRestoreRetentionDays(next);
    if (saved) {
      showStatus(t("settingsSavedInline"));
    }
  }

  async function updateSearchHotkey(next: string) {
    if (!isValidSearchHotkey(next)) {
      showStatus(t("hotkeyInvalid"), "warning");
      return;
    }

    const saved = await setSearchHotkey(next);
    const status = await tauriApi.getGlobalHotkeyStatus().catch(() => null);
    setGlobalHotkeyStatus(status);
    if (saved) {
      showStatus(t("hotkeySaved"));
      setIsRecordingHotkey(false);
      setRecordingHotkeyPreview("");
    }
  }

  function recordHotkeyFromEvent(event: globalThis.KeyboardEvent) {
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      setIsRecordingHotkey(false);
      setRecordingHotkeyPreview("");
      return;
    }

    const accelerator = acceleratorFromKeyboardEvent(event, platform);
    setRecordingHotkeyPreview(accelerator ? formatHotkeyLabel(accelerator, platform) : event.key);
    if (!accelerator) {
      showStatus(t("hotkeyInvalid"), "warning");
      return;
    }
    void updateSearchHotkey(accelerator);
  }

  function updateAISettings(partial: Partial<AISettings>) {
    if (isSavingAISettings) return;
    setAiSettingsSaveError(false);
    setAiConnectionStatus(null);
    setAiDebugStatus(null);
    setAiSettings((current) => current ? { ...current, ...partial } : current);
  }

  function applyClassificationPreset(mode: Exclude<AIClassificationPresetId, "custom">) {
    setAiSettingsSaveError(false);
    setAiConnectionStatus(null);
    setAiDebugStatus(null);
    setAiSettings((current) => current ? applyAIClassificationPreset(current, mode) : current);
  }

  const updateAIUserMode = useCallback((mode: AIUserMode) => {
    setAiSettingsSaveError(false);
    setAiConnectionStatus(null);
    setAiSettings((current) => {
      if (!current) return current;
      if (mode === "off") return { ...current, enabled: false };

      if (mode === "local" && current.provider === "ollama") return { ...current, enabled: true };
      if (mode === "cloud" && current.provider !== "ollama") return { ...current, enabled: true };

      const targetPreset = mode === "local"
        ? aiPresets.find((preset) => preset.providerKind === "ollama")
        : aiPresets.find((preset) => preset.providerKind !== "ollama" && preset.id === current.preset)
          ?? aiPresets.find((preset) => preset.providerKind !== "ollama" && preset.id === "deepseek")
          ?? aiPresets.find((preset) => preset.providerKind !== "ollama");
      return targetPreset ? applyProviderPreset({ ...current, enabled: true }, targetPreset) : { ...current, enabled: true };
    });
  }, [aiPresets]);

  useEffect(() => {
    if (!aiSettings || !aiPresets.length) return;
    let requestedMode: string | null = null;
    try {
      requestedMode = window.sessionStorage.getItem(AI_SETTINGS_MODE_REQUEST_KEY);
      if (requestedMode === "local" || requestedMode === "cloud") {
        window.sessionStorage.removeItem(AI_SETTINGS_MODE_REQUEST_KEY);
      } else {
        requestedMode = null;
      }
    } catch {
      requestedMode = null;
    }
    if (requestedMode === "local" || requestedMode === "cloud") {
      updateAIUserMode(requestedMode);
      focusSettingsSection("settings-ai-provider");
    }
  }, [aiPresets.length, aiSettings, focusSettingsSection, updateAIUserMode]);

  function selectAIPreset(presetId: AIProviderPresetId) {
    const preset = aiPresets.find((item) => item.id === presetId);
    setAiSettingsSaveError(false);
    setAiConnectionStatus(null);
    setAiDebugStatus(null);
    setAiSettings((current) => {
      if (!preset) return current ? { ...current, preset: presetId } : current;
      const fallback = current ?? defaultAISettingsFromPreset(preset);
      if (!fallback) return fallback;
      return applyProviderPreset(fallback, preset);
    });
  }

  function selectCustomProfile(profileId: string) {
    const profile = aiSettings?.customProfiles?.find((item) => item.id === profileId);
    if (!profile) return;
    updateAISettings({
      preset: "custom_openai_compatible",
      provider: "openai_compatible",
      activeCustomProfileId: profile.id,
      baseUrl: profile.baseUrl,
      chatPath: profile.chatPath,
      modelsPath: profile.modelsPath ?? null,
      model: profile.model,
      forceJsonOutput: profile.supportsResponseFormat,
      enableThinking: profile.supportsThinking,
      maxTokens: profile.maxOutputTokens,
      temperature: Math.min(profile.temperatureMax, Math.max(profile.temperatureMin, aiSettings?.temperature ?? 0)),
      extraBodyJson: profile.extraBodyJson ?? null,
      apiKey: "",
      apiKeyConfigured: profile.apiKeyConfigured ?? false,
      apiKeyAction: "preserve"
    });
  }

  function createCustomProfile() {
    if (!aiSettings) return;
    const id = `custom-${Date.now().toString(36)}`;
    const profile: AICustomProviderProfile = {
      id,
      name: t("aiCustomProfileDefaultName").replace("{number}", String((aiSettings.customProfiles?.length ?? 0) + 1)),
      baseUrl: aiSettings.baseUrl,
      chatPath: aiSettings.chatPath || "/chat/completions",
      modelsPath: aiSettings.modelsPath ?? "/models",
      model: aiSettings.model,
      supportsResponseFormat: aiSettings.forceJsonOutput,
      supportsThinking: aiSettings.enableThinking,
      thinkingParameter: "generic",
      tokenParameter: "max_tokens",
      contentPath: "choices[0].message.content",
      reasoningPath: "choices[0].message.reasoning_content",
      temperatureMin: 0,
      temperatureMax: 2,
      maxOutputTokens: aiSettings.maxTokens,
      extraBodyJson: aiSettings.extraBodyJson ?? null,
      apiKeyConfigured: false
    };
    updateAISettings({
      preset: "custom_openai_compatible",
      provider: "openai_compatible",
      customProfiles: [...(aiSettings.customProfiles ?? []), profile],
      activeCustomProfileId: id,
      apiKey: "",
      apiKeyConfigured: false,
      apiKeyAction: "preserve"
    });
  }

  function removeCustomProfile() {
    if (!aiSettings?.activeCustomProfileId) return;
    const profiles = (aiSettings.customProfiles ?? []).filter((profile) => profile.id !== aiSettings.activeCustomProfileId);
    updateAISettings({
      customProfiles: profiles,
      activeCustomProfileId: profiles[0]?.id ?? null,
      enabled: profiles.length > 0 ? aiSettings.enabled : false,
      ...(profiles[0] ? {
        baseUrl: profiles[0].baseUrl,
        chatPath: profiles[0].chatPath,
        modelsPath: profiles[0].modelsPath ?? null,
        model: profiles[0].model,
        apiKey: "",
        apiKeyConfigured: profiles[0].apiKeyConfigured ?? false,
        apiKeyAction: "preserve" as const
      } : {})
    });
  }

  function updateActiveCustomProfile(partial: Partial<AICustomProviderProfile>) {
    if (!aiSettings?.activeCustomProfileId) return;
    updateAISettings({
      customProfiles: (aiSettings.customProfiles ?? []).map((profile) => profile.id === aiSettings.activeCustomProfileId ? { ...profile, ...partial } : profile)
    });
  }

  function selectUserProvider(value: AIProviderPresetId | "openai_compatible" | "custom") {
    if (value !== "openai_compatible") {
      selectAIPreset(value === "custom" ? "custom_openai_compatible" : value);
      return;
    }
    const currentCloudPreset = aiSettings && aiSettings.provider !== "ollama" && aiSettings.preset !== "deepseek" && aiSettings.preset !== "custom_openai_compatible"
      ? aiSettings.preset
      : "custom_openai_compatible";
    selectAIPreset(currentCloudPreset as AIProviderPresetId);
  }

  async function saveAISettings() {
    if (!aiSettings || isSavingAISettings) return;
    const next = normalizeAISettingsForSave(aiSettings);
    const requestedKeyAction = next.apiKeyAction ?? (next.apiKey ? "replace" : "preserve");
    const requestId = ++aiSaveRequestRef.current;
    const previous = persistedAISettings;
    setAiSettings((current) => current ? { ...current, apiKey: "" } : current);
    setIsSavingAISettings(true);
    setAiSettingsSaveError(false);
    setAiConnectionStatus(null);
    try {
      await tauriApi.saveAISettings(next);
      const saved = await tauriApi.getAISettings();
      if (requestedKeyAction === "replace" && !saved.apiKeyConfigured) {
        throw new Error(t("aiCredentialReadbackFailed"));
      }
      if (requestId !== aiSaveRequestRef.current) return;
      setAiSettings(saved);
      setPersistedAISettings(saved);
      setAiSettingsSaveError(false);
      publishAIProcessingMode({ enabled: saved.enabled, provider: saved.provider });
      setAiConnectionStatus({
        tone: "success",
        role: "status",
        message: requestedKeyAction === "replace" ? t("aiCredentialSaved") : t("aiSettingsSaved")
      });
      setSecretRevealResetVersion((version) => version + 1);
    } catch (error) {
      if (requestId !== aiSaveRequestRef.current) return;
      setAiSettingsSaveError(true);
      setAiConnectionStatus({
        tone: "warning",
        role: "alert",
        message: sanitizeAIStatusMessage(`${t("aiSettingsSaveFailed")}：${localizedStableError(error, t)}`, next.apiKey || aiSettings.apiKey || previous?.apiKey || "")
      });
      setSecretRevealResetVersion((version) => version + 1);
    } finally {
      if (requestId === aiSaveRequestRef.current) setIsSavingAISettings(false);
    }
  }

  async function testAIConnection() {
    if (!aiSettings || isTestingAIConnection) return;
    const next = normalizeAISettingsForSave(aiSettings);
    setIsTestingAIConnection(true);
    setAiConnectionStatus(null);
    try {
      const result = await tauriApi.testAIProviderConnection(next);
      setAiConnectionStatus({
        tone: "success",
        role: "status",
        message: aiConnectionSuccessMessage(result, t("aiConnectionSucceeded"))
      });
    } catch (error) {
      setAiConnectionStatus({
        tone: "warning",
        role: "alert",
        message: sanitizeAIStatusMessage(`${t("aiConnectionTestFailed")}：${localizedStableError(error, t)}`, aiSettings.apiKey)
      });
    } finally {
      setIsTestingAIConnection(false);
    }
  }

  async function refreshAIModels() {
    if (!aiSettings || isDiscoveringAIModels) return;
    setIsDiscoveringAIModels(true);
    try {
      const models = await tauriApi.listAIModels(normalizeAISettingsForSave(aiSettings));
      setAiModels(models);
      setAiConnectionStatus({ tone: "success", role: "status", message: t("aiModelsLoaded").replace("{count}", String(models.length)) });
    } catch (error) {
      setAiConnectionStatus({
        tone: "warning",
        role: "alert",
        message: sanitizeAIStatusMessage(`${t("aiModelDiscoveryFailed")}：${localizedStableError(error, t)}`, aiSettings.apiKey)
      });
    } finally {
      setIsDiscoveringAIModels(false);
    }
  }

  async function refreshAITraces() {
    if (isLoadingAITraces) return;
    setIsLoadingAITraces(true);
    try {
      setAiTraces(await tauriApi.listAIRequestTraces());
    } catch (error) {
      setAiConnectionStatus({ tone: "warning", role: "alert", message: `${t("aiInspectorLoadFailed")}：${readableError(error)}` });
    } finally {
      setIsLoadingAITraces(false);
    }
  }

  async function clearAITraces() {
    await tauriApi.clearAIRequestTraces();
    setAiTraces([]);
  }

  async function exportAITraces() {
    try {
      const json = await tauriApi.exportAIRequestTraces();
      const blob = new Blob([json], { type: "application/json;charset=utf-8" });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = `fylura-ai-traces-${new Date().toISOString().replace(/[:.]/g, "-")}.json`;
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      setAiConnectionStatus({ tone: "warning", role: "alert", message: `${t("aiInspectorExportFailed")}：${readableError(error)}` });
    }
  }

  async function debugAIClassificationOnce() {
    if (!aiSettings || isDebuggingAI) return;
    const target = aiDebugTarget.trim();
    if (!target) {
      setAiDebugStatus({ tone: "warning", role: "alert", message: t("aiDebugMissingTarget") });
      return;
    }

    setIsDebuggingAI(true);
    setAiDebugStatus(null);
    setAiDebugResult(null);
    try {
      const result = await tauriApi.debugAIClassificationOnce(target);
      setAiDebugResult(result);
      setAiDebugStatus({
        tone: result.success ? "success" : "warning",
        role: result.success ? "status" : "alert",
        message: result.success
          ? t("aiDebugSucceeded")
          : `${t("aiDebugFinished")}：${result.parseError ?? t("aiDebugParseFailed")}`
      });
    } catch (error) {
      setAiDebugStatus({
        tone: "warning",
        role: "alert",
        message: sanitizeAIStatusMessage(`${t("aiDebugFinished")}：${readableError(error)}`, aiSettings.apiKey)
      });
    } finally {
      setIsDebuggingAI(false);
    }
  }

  return (
    <>
    <div
      aria-busy={isLoadingSettings}
      aria-disabled={isLoadingSettings}
      inert={isLoadingSettings ? true : undefined}
      className="contents"
    >
      <SettingsLayout
        sections={settingsSections}
        activeSectionId={activeSettingsSection}
        sectionLabel={t("settingsSectionsLabel")}
        onSectionChange={(sectionId, options) => focusSettingsSection(sectionId, options)}
        scrollRef={settingsScrollRef}
        header={(
          <SettingsSearch
            containerRef={settingsScrollRef}
            sections={settingsSections}
            localeKey={language}
            label={t("settingsSearchLabel")}
            placeholder={t("settingsSearchPlaceholder")}
            clearLabel={t("settingsSearchClear")}
            noResultsLabel={t("settingsSearchNoResults")}
            resultCountLabel={(count, query) => t("settingsSearchResultCount").replace("{count}", String(count)).replace("{query}", query)}
            onSelect={(result) => {
              focusSettingsSection(result.sectionId, { focusContent: false });
              if (!result.targetId) return;
              const focusTarget = (attempt = 0) => {
                const target = settingsScrollRef.current?.querySelector<HTMLElement>(`#${result.targetId}`);
                if (!target) return;
                const disclosure = target.closest<HTMLDetailsElement>("details");
                if (disclosure && !disclosure.open) {
                  disclosure.querySelector<HTMLElement>("summary")?.click();
                  if (attempt < 2) {
                    window.requestAnimationFrame(() => focusTarget(attempt + 1));
                    return;
                  }
                }
                const targetControl = target.matches("button, input, select, textarea, [tabindex]")
                  ? target
                  : target.querySelector<HTMLElement>("button, input, select, textarea, [tabindex]");
                const targetToFocus = targetControl?.matches("[data-settings-select-native]")
                  ? targetControl.parentElement?.querySelector<HTMLElement>("[data-settings-select-trigger]")
                  : targetControl;
                const focusableTarget = targetToFocus?.matches(":disabled")
                  ? target.closest<HTMLDetailsElement>("details")?.querySelector<HTMLElement>("summary")
                    ?? target.closest<HTMLElement>("[data-settings-section-content]")?.querySelector<HTMLElement>("[data-settings-section-heading]")
                  : targetToFocus;
                focusableTarget?.scrollIntoView({ block: "nearest" });
                focusableTarget?.focus({ preventScroll: true });
              };
              window.requestAnimationFrame(() => focusTarget());
            }}
          />
        )}
      >
        <div className="grid gap-2">
          <p className="max-w-2xl text-sm leading-6 text-[var(--zc-text-secondary)]">{t("settingsDesc")}</p>
          {settingsStatus ? <SettingsInlineMessage tone={settingsStatusTone === "warning" ? "warning" : "success"} role={settingsStatusTone === "warning" ? "alert" : "status"}>{settingsStatus}</SettingsInlineMessage> : null}
        </div>

        <GeneralSettingsSection
          t={t}
          language={language}
          onLanguage={setLanguage}
          theme={theme}
          onTheme={setTheme}
          density={density}
          onDensity={setDensity}
          aiSettings={aiSettings}
          onOpenAISettings={() => focusSettingsSection("settings-ai")}
          folderNamingLanguage={folderNamingLanguage}
          onFolderNamingLanguage={(next) => void updateFolderNamingLanguage(next)}
          closeBehavior={closeBehavior}
          onCloseBehavior={(next) => void updateCloseBehavior(next)}
          backgroundIndexOnStartup={backgroundIndexOnStartup}
          onBackgroundIndexOnStartup={(next) => void updateBackgroundIndexOnStartup(next)}
          launchAtLogin={launchAtLogin}
          onLaunchAtLogin={(next) => void updateLaunchAtLogin(next)}
        />

        <FileSourcesSettingsSection
          t={t}
          defaultScanFolders={defaultScanFolders}
          watcherRootStatuses={watcherRootStatuses}
          organizeRootMode={organizeRootMode}
          organizeRootPath={organizeRootPath ?? null}
          onAddScanFolder={() => void addDefaultScanFolder()}
          onSetScanRootEnabled={(root, next) => void setScanRootEnabled(root, next)}
          onScanRootNow={(root) => void scanRootNow(root)}
          onRequestDelete={setFolderDeleteConfirm}
          onOrganizeRootMode={(next) => void updateOrganizeRootMode(next)}
          onOrganizeRootPath={(next) => void updateOrganizeRootPath(next)}
          onChooseOrganizeRootPath={() => void chooseOrganizeRootPath()}
        />

        <GlobalSearchSettingsSection
          t={t}
          platform={platform}
          searchHotkey={searchHotkey}
          hotkey={hotkey}
          isRecordingHotkey={isRecordingHotkey}
          recordingHotkeyPreview={recordingHotkeyPreview}
          hotkeyCaptureRef={hotkeyCaptureRef}
          globalHotkeyError={globalHotkeyError}
          globalHotkeyStatus={globalHotkeyStatus}
          searchScopeMode={searchScopeMode}
          customSearchRoots={customSearchRoots}
          pendingBackgroundRoots={pendingBackgroundRoots}
          currentBackgroundRoot={currentBackgroundRoot}
          isBackgroundIndexing={isBackgroundIndexing}
          completedBackgroundRoots={completedBackgroundRoots}
          failedBackgroundRoots={failedBackgroundRoots}
          onStartRecording={() => setIsRecordingHotkey(true)}
          onUpdateHotkey={(next) => void updateSearchHotkey(next)}
          onSearchScopeMode={(next) => void updateSearchScopeMode(next)}
          onAddSearchRoot={() => void addCustomSearchRoot()}
          onSetSearchRootEnabled={(root, next) => void setSearchRootEnabled(root, next)}
          onIndexSearchRootNow={indexSearchRootNow}
          onRequestDelete={setFolderDeleteConfirm}
          backgroundRootState={backgroundRootState}
        />

        <GlobalIndexSettingsSection
          t={t}
          status={globalIndexStatus}
          sources={globalIndexSources}
          loadState={globalIndexLoadState}
          loadError={globalIndexLoadError}
          isLoading={isLoadingGlobalIndex}
          isUpdating={isUpdatingGlobalIndex}
          statusText={(status) => globalIndexStatusText(status, t)}
          providerStatusText={(status) => globalIndexProviderStatusText(status, t)}
          errorText={(error) => globalIndexErrorText(error, t)}
          onAction={(action, message) => void runGlobalIndexAction(action, message)}
        />

        <PlatformDiagnosticsSettingsSection
          t={t}
          capabilities={runtimeCapabilities}
          globalIndexStatus={globalIndexStatus}
          globalIndexSources={globalIndexSources}
          globalIndexLoadState={globalIndexLoadState}
          statusText={(status) => globalIndexStatusText(status, t)}
        />

        <ManagedLibrarySettingsSection
          t={t}
          scopes={managedScopes}
          aiManagementStatus={aiManagementStatus}
          managedScopePath={managedScopePath}
          onManagedScopePath={setManagedScopePath}
          isUpdating={isUpdatingGlobalIndex}
          policyText={(summary) => managedScopePolicyText(summary, t)}
          onAdd={() => void addManagedScopeFromSettings()}
          onUpdate={(scope, patch) => void updateManagedScope(scope, patch)}
          onRemove={(scope) => void removeManagedScope(scope)}
        />

        <AutomationSettingsSection t={t} onOpenAutomation={() => setView("automation")} />

        <AISettingsSection t={t}>
          {isLoadingAISettings || !aiSettings ? (
            <SettingsEmptyState title={t("aiSettingsLoading")} description={t("aiSettingsLoadingDesc")} />
          ) : (
            <fieldset
              disabled={isSavingAISettings}
              aria-busy={isSavingAISettings}
              className="grid min-w-0 gap-0"
              data-ai-runtime-status={runtimeAIStatus}
              data-ai-runtime-mode={runtimeAIUserMode}
              data-ai-saved-mode={persistedAISettings ? aiUserMode(persistedAISettings) : "loading"}
              data-ai-draft-mode={draftAIUserMode}
              data-ai-dirty={aiSettingsDirty}
              data-ai-saving={isSavingAISettings}
            >
              <div
                data-ai-save-bar
                className="sticky top-[53px] z-10 grid min-w-0 gap-2 border-y border-[var(--zc-divider)] bg-[var(--zc-surface)] py-3 min-[841px]:top-4 min-[841px]:grid-cols-[minmax(0,1fr)_auto] min-[841px]:items-center"
              >
                <div className="grid min-w-0 gap-1">
                  <strong className="text-sm text-[var(--zc-text-primary)]">
                    {t("aiCurrentActiveMode").replace("{mode}", t(runtimeAIUserMode === "off" ? "modeAIDisabled" : runtimeAIUserMode === "local" ? "modeAILocal" : "modeAICloud"))}
                  </strong>
                  {aiSettingsDirty ? (
                    <span className="text-xs leading-5 text-[var(--zc-warning-text)]" data-ai-unsaved-draft>
                      {t("aiUnsavedDraftMode").replace("{mode}", t(draftAIUserMode === "off" ? "modeAIDisabled" : draftAIUserMode === "local" ? "modeAILocal" : "modeAICloud"))}
                    </span>
                  ) : null}
                </div>
                <button type="button" className={buttonSecondary} onClick={() => void saveAISettings()} disabled={!aiSettingsDirty || isSavingAISettings || isTestingAIConnection}>
                  {isSavingAISettings ? t("aiSavingSettings") : t("aiSaveSettings")}
                </button>
              </div>
              {aiConnectionStatus ? (
                <div data-ai-status-region className="min-w-0 py-3">
                  <SettingsInlineMessage tone={aiConnectionStatus.tone} role={aiConnectionStatus.role}>
                    <span className="block min-w-0 [overflow-wrap:anywhere]">{aiConnectionStatus.message}</span>
                    {aiSettingsSaveError ? (
                      <span className="mt-1 block text-xs leading-5" data-ai-settings-state="error">
                        {t("aiSettingsRetainedAfterFailure")}
                      </span>
                    ) : null}
                  </SettingsInlineMessage>
                </div>
              ) : null}
              <SettingsRow controlWidth="wide" label={t("aiModeLabel")} description={t(aiUserMode(aiSettings) === "off" ? "modeAIDisabledDesc" : aiUserMode(aiSettings) === "local" ? "modeAILocalDesc" : "modeAICloudDesc")}>
                <SettingsSegmentedControl
                  value={aiUserMode(aiSettings)}
                  ariaLabel={t("aiModeLabel")}
                  layout="three-option-responsive"
                  options={[
                    { value: "off", label: t("modeAIDisabled") },
                    { value: "local", label: t("modeAILocal") },
                    { value: "cloud", label: t("modeAICloud") }
                  ]}
                  onChange={updateAIUserMode}
                />
              </SettingsRow>
              <SettingsSwitch
                id="settings-ai-cleanup"
                label={t("aiCleanupEnabledLabel")}
                description={t("aiCleanupEnabledDesc")}
                checked={aiSettings.cleanupAiEnabled}
                disabled={aiDependentControlsDisabled}
                onChange={(next) => updateAISettings({ cleanupAiEnabled: next })}
              />
              <SettingsSwitch
                id="settings-ai-cleanup-local-consent"
                label={t("aiCleanupLocalConsentLabel")}
                description={t("aiCleanupLocalConsentDesc")}
                checked={aiSettings.cleanupLocalAiAllowed}
                disabled={aiDependentControlsDisabled || !aiSettings.cleanupAiEnabled}
                onChange={(next) => updateAISettings({ cleanupLocalAiAllowed: next })}
              />
              <SettingsSwitch
                id="settings-ai-cleanup-cloud-consent"
                label={t("aiCleanupCloudConsentLabel")}
                description={t("aiCleanupCloudConsentDesc")}
                checked={aiSettings.cleanupCloudAiAllowed}
                disabled={aiDependentControlsDisabled || !aiSettings.cleanupAiEnabled}
                onChange={(next) => updateAISettings({ cleanupCloudAiAllowed: next })}
              />
              <SettingsSelect
                id="settings-ai-provider"
                label={t("aiProviderPreset")}
                description={t("aiProviderPresetDesc")}
                value={userProviderValue(aiSettings)}
                disabled={aiDependentControlsDisabled}
                options={[
                  { value: "deepseek" as const, label: t("aiProviderDeepSeek") },
                  { value: "openai_compatible" as const, label: t("aiProviderOpenaiCompatible") },
                  ...aiPresets
                    .filter((preset) => !["deepseek", "ollama", "custom_openai_compatible"].includes(preset.id))
                    .map((preset) => ({ value: preset.id, label: preset.label })),
                  { value: "ollama" as const, label: t("aiProviderOllama") },
                  { value: "custom" as const, label: t("aiProviderCustom") }
                ]}
                onChange={selectUserProvider}
              />
              <SettingsRow label={t("aiClassificationPresets")} description={t("aiClassificationPresetsDesc")}>
                <SettingsSegmentedControl
                  value={activeAIClassificationPreset}
                  ariaLabel={t("aiClassificationPresets")}
                  options={AI_CLASSIFICATION_PRESET_IDS.map((presetId) => ({ value: presetId, label: t(AI_CLASSIFICATION_LABEL_KEYS[presetId]) }))}
                  disabled={aiDependentControlsDisabled}
                  onChange={(presetId) => { if (presetId !== "custom") applyClassificationPreset(presetId); }}
                />
              </SettingsRow>
              <SettingsInlineMessage tone={aiSettings.enabled && aiSettings.provider !== "ollama" && !aiSettings.apiKeyConfigured ? "warning" : "info"}>
                {aiSettings.enabled && aiSettings.provider !== "ollama" && !aiSettings.apiKeyConfigured
                  ? t("aiConfigurationIncomplete")
                  : aiSettings.enabled
                    ? t(aiUserMode(aiSettings) === "local" ? "modeAILocalDesc" : "modeAICloudDesc")
                    : t("aiDisabledConfigurationPreserved")}
              </SettingsInlineMessage>
              <SettingsDisclosure
                title={t("advancedSettings")}
                description={developerMode ? t("developerModeDesc") : t("aiAdvancedConnectionDesc")}
                open={aiAdvancedOpen}
                onOpenChange={setAiAdvancedOpen}
              >
                  <SettingsControlGroup title={t("aiAdvancedConnection")} description={t("aiAdvancedConnectionDesc")}>
                    <div data-ai-advanced-connection-grid className="grid min-w-0 gap-4 min-[841px]:grid-cols-2">
                      <SettingsTextField id="settings-ai-base-url" label={t("aiBaseUrlLabel")} value={aiSettings.baseUrl} maxLength={2048} disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ baseUrl: value })} />
                      <SettingsTextField id="settings-ai-chat-path" label={t("aiChatPathLabel")} value={aiSettings.chatPath} maxLength={512} disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ chatPath: value })} />
                      {aiSettings.provider === "ollama" ? (
                        <span className={quietText}>{t("aiLocalApiKeyHint")}</span>
                      ) : (
                        <div className="grid min-w-0 gap-2">
                          <SettingsSecretField
                            id="settings-ai-api-key"
                            label={t("aiApiKeyLabel")}
                            value={aiSettings.apiKey}
                            disabled={aiDependentControlsDisabled}
                            showLabel={t("showApiKey")}
                            hideLabel={t("hideApiKey")}
                            onChange={(value) => updateAISettings({ apiKey: value, apiKeyAction: value ? "replace" : "preserve" })}
                            placeholder={aiSettings.apiKeyConfigured ? t("aiStoredApiKeyPlaceholder") : t("aiEmptyApiKeyPlaceholder")}
                            resetKey={`${activeSettingsSection}:${draftAIUserMode}:${developerMode}:${aiAdvancedOpen}:${aiSettings.provider}:${aiSettings.preset}:${secretRevealResetVersion}`}
                          />
                          {aiSettings.apiKeyConfigured ? (
                            <button className={buttonSecondary} type="button" disabled={aiDependentControlsDisabled} onClick={() => updateAISettings({ apiKey: "", apiKeyAction: "clear" })}>
                              {t("aiClearApiKey")}
                            </button>
                          ) : null}
                        </div>
                      )}
                      <SettingsTextField id="settings-ai-model" label={t("aiModelLabel")} value={aiSettings.model} maxLength={200} list="settings-ai-model-options" disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ model: value })} />
                      {aiSettings.preset === "custom_openai_compatible" ? (
                        <div className="grid min-w-0 gap-2 min-[841px]:col-span-2">
                          <SettingsSelect
                            id="settings-ai-custom-profile"
                            label={t("aiCustomProfileLabel")}
                            description={t("aiCustomProfileDesc")}
                            value={aiSettings.activeCustomProfileId ?? ""}
                            options={[
                              ...(aiSettings.customProfiles ?? []).map((profile) => ({ value: profile.id, label: `${profile.name}${profile.apiKeyConfigured ? " · ✓" : ""}` })),
                              { value: "", label: t("aiCustomProfileNone") }
                            ]}
                            onChange={(value) => { if (value) selectCustomProfile(value); }}
                          />
                          <div className="flex flex-wrap gap-2">
                            <button className={buttonSecondary} type="button" onClick={createCustomProfile} disabled={aiDependentControlsDisabled}>{t("aiCreateCustomProfile")}</button>
                            <button className={buttonSecondary} type="button" onClick={removeCustomProfile} disabled={aiDependentControlsDisabled || !aiSettings.activeCustomProfileId}>{t("aiDeleteCustomProfile")}</button>
                          </div>
                          {aiSettings.activeCustomProfileId ? (
                            <SettingsTextField
                              id="settings-ai-custom-profile-name"
                              label={t("aiCustomProfileNameLabel")}
                              value={aiSettings.customProfiles?.find((profile) => profile.id === aiSettings.activeCustomProfileId)?.name ?? ""}
                              maxLength={120}
                              disabled={aiDependentControlsDisabled}
                              onChange={(value) => updateActiveCustomProfile({ name: value })}
                            />
                          ) : null}
                        </div>
                      ) : null}
                      <datalist id="settings-ai-model-options">
                        {[
                          ...(aiPresets.find((preset) => preset.id === aiSettings.preset)?.suggestedModels ?? []),
                          ...aiModels.map((model) => model.id)
                        ].filter((model, index, models) => model && models.indexOf(model) === index).map((model) => <option key={model} value={model} />)}
                      </datalist>
                      <SettingsTextField id="settings-ai-models-path" label={t("aiModelsPathLabel")} description={t("aiModelsPathDesc")} value={aiSettings.modelsPath ?? ""} maxLength={512} disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ modelsPath: value || null })} />
                      <div className="flex items-end">
                        <button className={buttonSecondary} type="button" onClick={() => void refreshAIModels()} disabled={aiDependentControlsDisabled || isDiscoveringAIModels}>
                          {isDiscoveringAIModels ? t("aiDiscoveringModels") : t("aiRefreshModels")}
                        </button>
                      </div>
                    </div>
                  </SettingsControlGroup>
                  {developerMode ? (<>
                    <SettingsControlGroup title={t("aiAdvancedPerformance")} description={t("aiAdvancedPerformanceDesc")}>
                    <div className="grid min-w-0 gap-4 min-[841px]:grid-cols-2">
                      <SettingsTextField id="settings-ai-batch-size" label={t("aiBatchSizeLabel")} description={t("aiBatchSizeDesc")} type="number" value={String(aiSettings.batchSize)} min={1} max={100} disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ batchSize: Math.min(100, Math.max(1, Number(value) || 1)) })} />
                      <SettingsTextField id="settings-ai-concurrency" label={t("aiConcurrencyLabel")} description={t("aiConcurrencyDesc")} type="number" value={String(aiSettings.classificationConcurrency)} min={1} max={4} disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ classificationConcurrency: Math.min(4, Math.max(1, Number(value) || 1)) })} />
                      <SettingsTextField id="settings-ai-max-tokens" label={t("aiMaxTokensLabel")} type="number" value={String(aiSettings.maxTokens)} min={512} max={32768} disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ maxTokens: Math.min(32768, Math.max(512, Number(value) || 512)) })} />
                      <SettingsTextField id="settings-ai-timeout" label={t("aiTimeoutLabel")} type="number" value={String(aiSettings.timeoutSeconds)} min={1} max={600} disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ timeoutSeconds: Math.min(600, Math.max(1, Number(value) || 1)) })} />
                    </div>
                  </SettingsControlGroup>
                  <SettingsControlGroup title={t("aiAdvancedPrivacy")} description={t("aiAdvancedPrivacyDesc")}>
                    <SettingsSwitch id="settings-ai-learned-rules" label={t("aiLearnedRulesLabel")} description={t("aiLearnedRulesDesc")} checked={useLearnedRulesAsAutoRules} disabled={aiDependentControlsDisabled} onChange={(next) => void updateSettings({ useLearnedRulesAsAutoRules: next })} />
                    <SettingsSwitch id="settings-ai-legacy-rules" label={t("aiLegacyRulesLabel")} description={t("aiLegacyRulesDesc")} checked={useLegacyBuiltinClassificationRules} disabled={aiDependentControlsDisabled} onChange={(next) => void updateSettings({ useLegacyBuiltinClassificationRules: next })} />
                    <SettingsSwitch id="settings-ai-force-json" label={t("aiForceJsonLabel")} description={t("aiForceJsonDesc")} checked={aiSettings.forceJsonOutput} disabled={aiDependentControlsDisabled} onChange={(next) => updateAISettings({ forceJsonOutput: next })} />
                    <SettingsSwitch id="settings-ai-thinking" label={t("aiThinkingLabel")} description={t("aiThinkingDesc")} checked={aiSettings.enableThinking} disabled={aiDependentControlsDisabled} onChange={(next) => updateAISettings({ enableThinking: next })} />
                    <SettingsSwitch id="settings-ai-send-full-path" label={t("aiSendFullPathLabel")} description={t("aiSendFullPathDesc")} checked={aiSettings.sendFullPath} disabled={aiDependentControlsDisabled} onChange={(next) => updateAISettings({ sendFullPath: next })} />
                    <SettingsSwitch id="settings-ai-send-parent-path" label={t("aiSendParentPathLabel")} description={t("aiSendParentPathDesc")} checked={aiSettings.sendParentPath} disabled={aiDependentControlsDisabled} onChange={(next) => updateAISettings({ sendParentPath: next })} />
                    <SettingsTextField id="settings-ai-reasoning-effort" label={t("aiReasoningEffortLabel")} value={aiSettings.reasoningEffort ?? ""} maxLength={64} disabled={aiDependentControlsDisabled} onChange={(value) => updateAISettings({ reasoningEffort: value || null })} placeholder={t("aiReasoningPlaceholder")} />
                    <label className="grid min-w-0 gap-1.5">
                      <span className="text-sm font-medium text-[var(--zc-text-primary)]">{t("aiExtraBodyLabel")}</span>
                      <textarea className={cn(settingsField, "min-h-24 resize-y py-2 font-mono")} value={aiSettings.extraBodyJson ?? ""} maxLength={16384} disabled={aiDependentControlsDisabled} onChange={(event) => updateAISettings({ extraBodyJson: event.target.value || null })} placeholder={t("aiExtraBodyPlaceholder")} />
                      <span className={quietText}>{t("aiSecretsHint")}</span>
                    </label>
                  </SettingsControlGroup>
                  </>) : null}
                  <DeveloperDiagnosticsSection
                    t={t}
                    diagnosticsMode={aiSettings.diagnosticsMode}
                    onDiagnosticsMode={(mode) => updateAISettings({ diagnosticsMode: mode })}
                    includeSensitiveDocumentContentInDiagnostics={Boolean(aiSettings.includeSensitiveDocumentContentInDiagnostics)}
                    onIncludeSensitiveDocumentContentInDiagnostics={(enabled) => updateAISettings({ includeSensitiveDocumentContentInDiagnostics: enabled })}
                    aiTraces={aiTraces}
                    isLoadingAITraces={isLoadingAITraces}
                    onRefreshAITraces={() => void refreshAITraces()}
                    onExportAITraces={() => void exportAITraces()}
                    onClearAITraces={() => void clearAITraces()}
                    developerMode={developerMode}
                    aiDebugAvailable={Boolean(runtimeCapabilities?.aiDebugAvailable)}
                    selectedLibraryFile={selectedLibraryFile}
                    aiDebugTarget={aiDebugTarget}
                    onAiDebugTarget={setAiDebugTarget}
                    aiDependentControlsDisabled={aiDependentControlsDisabled}
                    isDebuggingAI={isDebuggingAI}
                    aiDebugStatus={aiDebugStatus}
                    aiDebugResult={aiDebugResult}
                    apiKey={aiSettings.apiKey}
                    onUseSelectedFile={() => setAiDebugTarget(selectedLibraryFile?.id ?? "")}
                    onDebug={() => void debugAIClassificationOnce()}
                  />
                  {aiSettings.preset === "deepseek" && ["deepseek-chat", "deepseek-reasoner"].includes(aiSettings.model.trim()) ? <SettingsInlineMessage tone="warning">{t("aiOldModelWarning")}</SettingsInlineMessage> : null}
                  {(aiSettings.provider === "ollama" || aiSettings.model.toLowerCase().includes("qwen3")) ? <SettingsInlineMessage tone="warning">{t("aiQwenWarning")}</SettingsInlineMessage> : null}
                  <div className="flex flex-wrap justify-end gap-2">
                    <button className={buttonSecondary} onClick={() => void testAIConnection()} disabled={aiDependentControlsDisabled || isTestingAIConnection || isSavingAISettings}>
                      {isTestingAIConnection ? t("aiTestingConnection") : t("aiTestConnection")}
                    </button>
                  </div>
              </SettingsDisclosure>
            </fieldset>
          )}
        </AISettingsSection>

        <PrivacyContentSettingsSection
          t={t}
          restoreRetentionDays={restoreRetentionDays}
          onRestoreRetentionDays={(next) => void updateRestoreRetentionDays(next)}
        />

        <AboutSettingsSection t={t} developerMode={developerMode} onDeveloperMode={setDeveloperModePreference} />
      </SettingsLayout>
    </div>
    <ConfirmDialog
      open={Boolean(folderDeleteConfirm)}
      tone="warning"
      title={folderDeleteConfirm?.kind === "scan" ? t("confirmDeleteScanFolderTitle") : t("confirmDeleteSearchFolderTitle")}
      description={
        folderDeleteConfirm
          ? (folderDeleteConfirm.kind === "scan" ? t("confirmDeleteScanFolderDesc") : t("confirmDeleteSearchFolderDesc")).replace("{path}", folderDeleteConfirm.root.path)
          : undefined
      }
      confirmLabel={folderDeleteConfirm?.kind === "scan" ? t("deleteScanFolder") : t("deleteSearchFolder")}
      cancelLabel={t("cancel")}
      isProcessing={isDeletingFolderConfig}
      onCancel={() => setFolderDeleteConfirm(null)}
      onConfirm={confirmFolderDelete}
    />
    </>
  );
}

function normalizeSettingsRoot(path: string) {
  return normalizePathLike(path.trim());
}

function defaultAISettingsFromPreset(preset?: AIProviderPreset): AISettings | null {
  if (!preset) return null;
  return {
    enabled: false,
    provider: preset.providerKind,
    preset: preset.id,
    baseUrl: preset.defaultBaseUrl,
    chatPath: preset.defaultChatPath || "/chat/completions",
    modelsPath: preset.modelsPath ?? null,
    apiKey: "",
    apiKeyAction: "preserve",
    model: preset.defaultModel,
    temperature: 0,
    maxTokens: 8192,
    batchSize: 10,
    classificationConcurrency: 2,
    timeoutSeconds: 120,
    sendFullPath: false,
    sendParentPath: true,
    classificationMode: "ai_first",
    cleanupAiEnabled: true,
    cleanupLocalAiAllowed: false,
    cleanupCloudAiAllowed: false,
    forceJsonOutput: true,
    enableThinking: false,
    reasoningEffort: null,
    extraBodyJson: null,
    diagnosticsMode: "off",
    includeSensitiveDocumentContentInDiagnostics: false
  };
}

function normalizeAISettingsForSave(settings: AISettings): AISettings {
  const chatPath = settings.chatPath.trim();
  const modelsPath = settings.modelsPath?.trim() || "";
  const baseUrl = settings.baseUrl.trim().replace(/\/+$/g, "");
  const normalizedChatPath = chatPath ? `/${chatPath.replace(/^[/]+/g, "")}` : "/chat/completions";
  const normalizedModelsPath = modelsPath ? `/${modelsPath.replace(/^[/]+/g, "")}` : null;
  const customProfiles = (settings.customProfiles ?? []).map((profile) => profile.id === settings.activeCustomProfileId
    ? {
        ...profile,
        baseUrl,
        chatPath: normalizedChatPath,
        modelsPath: normalizedModelsPath,
        model: settings.model.trim(),
        supportsResponseFormat: settings.forceJsonOutput,
        supportsThinking: settings.enableThinking,
        maxOutputTokens: Math.min(32768, Math.max(512, Math.floor(settings.maxTokens || 512))),
        extraBodyJson: settings.extraBodyJson?.trim() || null
      }
    : profile);
  return {
    ...settings,
    baseUrl,
    chatPath: normalizedChatPath,
    modelsPath: normalizedModelsPath,
    apiKey: settings.apiKey.trim(),
    model: settings.model.trim(),
    batchSize: Math.min(100, Math.max(1, Math.floor(settings.batchSize || 1))),
    classificationConcurrency: settings.provider === "ollama"
      ? 1
      : Math.min(4, Math.max(1, Math.floor(settings.classificationConcurrency || 1))),
    timeoutSeconds: Math.min(600, Math.max(1, Math.floor(settings.timeoutSeconds || 1))),
    maxTokens: Math.min(32768, Math.max(512, Math.floor(settings.maxTokens || 512))),
    reasoningEffort: settings.reasoningEffort?.trim() || null,
    extraBodyJson: settings.extraBodyJson?.trim() || null,
    diagnosticsMode: settings.diagnosticsMode ?? "off",
    includeSensitiveDocumentContentInDiagnostics: settings.includeSensitiveDocumentContentInDiagnostics ?? false,
    customProfiles,
    activeCustomProfileId: settings.activeCustomProfileId ?? null
  };
}

function sanitizeAIStatusMessage(message: string, apiKey: string) {
  const trimmed = apiKey.trim();
  return trimmed ? message.split(trimmed).join("[redacted]") : message;
}

function aiConnectionSuccessMessage(result: AIConnectionTestResult, successLabel: string) {
  const provider = result.provider === "ollama" ? "Ollama" : "OpenAI-compatible";
  const model = result.model || "—";
  return `${successLabel}: ${provider} / ${model} / ${result.elapsedMs}ms`;
}
