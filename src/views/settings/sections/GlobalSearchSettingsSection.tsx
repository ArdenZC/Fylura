import { requestSettingsSection } from "../../../components/spotlight/commandRegistry";
import { FolderPlus, Keyboard, Play, Trash2 } from "lucide-react";
import type { GlobalHotkeyStatus } from "../../../api/tauriApi";
import type { SearchRootSetting, SearchScopeMode } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonIconDanger, buttonSecondary, cn, focusVisibleState, buttonDefault, selectedFocusVisibleState, selectedSurface } from "../../../utils/tw";
import { compactPath } from "../../../utils/viewHelpers";
import { formatHotkeyLabel } from "../../../utils/hotkeys";
import { compactInteractiveRow, quietText } from "../../shared/ui";
import { SettingsEmptyState, SettingsInlineMessage, SettingsRow, SettingsSection, SettingsSegmentedControl, SettingsSwitchControl } from "../components/SettingsPrimitives";
import type { BackgroundRootState, FolderDeleteConfirmState } from "./settingsSectionTypes";

export interface GlobalSearchSettingsSectionProps {
  t: Translator;
  platform: NodeJS.Platform | "browser";
  searchHotkey: string;
  hotkey: string;
  isRecordingHotkey: boolean;
  recordingHotkeyPreview: string;
  hotkeyCaptureRef: React.RefObject<HTMLDivElement | null>;
  globalHotkeyError: string;
  globalHotkeyStatus: GlobalHotkeyStatus | null;
  searchScopeMode: SearchScopeMode;
  customSearchRoots: SearchRootSetting[];
  pendingBackgroundRoots: string[];
  currentBackgroundRoot: string | null;
  isBackgroundIndexing: boolean;
  completedBackgroundRoots: string[];
  failedBackgroundRoots: Array<{ path: string; message: string }>;
  onStartRecording: () => void;
  onUpdateHotkey: (value: string) => void;
  onSearchScopeMode: (value: SearchScopeMode) => void;
  onAddSearchRoot: () => void;
  onSetSearchRootEnabled: (root: SearchRootSetting, enabled: boolean) => void;
  onIndexSearchRootNow: (root: SearchRootSetting) => void;
  onRequestDelete: (state: FolderDeleteConfirmState) => void;
  backgroundRootState: (root: SearchRootSetting) => BackgroundRootState;
}

export function GlobalSearchSettingsSection({
  t,
  platform,
  searchHotkey,
  hotkey,
  isRecordingHotkey,
  recordingHotkeyPreview,
  hotkeyCaptureRef,
  globalHotkeyError,
  globalHotkeyStatus,
  searchScopeMode,
  customSearchRoots,
  pendingBackgroundRoots,
  currentBackgroundRoot,
  isBackgroundIndexing,
  completedBackgroundRoots,
  failedBackgroundRoots,
  onStartRecording,
  onUpdateHotkey,
  onSearchScopeMode,
  onAddSearchRoot,
  onSetSearchRootEnabled,
  onIndexSearchRootNow,
  onRequestDelete,
  backgroundRootState
}: GlobalSearchSettingsSectionProps) {
  return (
    <SettingsSection id="settings-search" title={t("settingsSearch")} description={t("settingsSearchDesc")}>
      <SettingsRow label={t("searchHotkey")} description={t("searchHotkeyDesc")}>
        <div className="flex flex-wrap items-center justify-start gap-2 min-[841px]:justify-end">
          <kbd className="rounded-[var(--zc-radius-control)] border border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] px-3 py-2 text-sm font-medium text-[var(--zc-text-primary)]">{hotkey}</kbd>
          <button className={cn(buttonSecondary, isRecordingHotkey && selectedFocusVisibleState, isRecordingHotkey && selectedSurface)} onClick={onStartRecording}>
            <Keyboard size={14} />
            <span>{t("changeHotkey")}</span>
          </button>
        </div>
      </SettingsRow>
      {isRecordingHotkey ? (
        <SettingsInlineMessage role="status">
          <div ref={hotkeyCaptureRef} className={cn("mt-2 grid gap-2 rounded-xl border border-[var(--zc-info-border)] bg-[var(--zc-info-soft)] px-3 py-3", focusVisibleState)} tabIndex={0}>
            <span>{t("recordingHotkey")}</span>
            <span className={quietText}>{t("hotkeyCaptureCurrent")}: {recordingHotkeyPreview || hotkey}</span>
            <span className={quietText}>{t("settingsEscapeKey")}: {t("cancel")}</span>
          </div>
        </SettingsInlineMessage>
      ) : null}
      {globalHotkeyError ? <SettingsInlineMessage tone="warning" role="alert">{t("hotkeyConflictHint")}</SettingsInlineMessage> : <span className={quietText}>{t("hotkeyActiveHint")}</span>}
      {globalHotkeyStatus ? <span className={quietText}>{t("hotkeyCaptureCurrent")}: {formatHotkeyLabel(globalHotkeyStatus.requestedAccelerator, platform)} {" · "} {t("hotkeyActiveHint")}: {globalHotkeyStatus.effectiveAccelerator ? formatHotkeyLabel(globalHotkeyStatus.effectiveAccelerator, platform) : t("globalIndexStatusUnavailable")}</span> : null}
      <div className="flex flex-wrap gap-2">
        {["CmdOrCtrl+K", "CmdOrCtrl+Shift+K", "Alt+Space", "CmdOrCtrl+Alt+Space"].map((accelerator) => (
          <button className={cn(buttonDefault, searchHotkey === accelerator && selectedFocusVisibleState, searchHotkey === accelerator && selectedSurface)} key={accelerator} aria-pressed={searchHotkey === accelerator} onClick={() => onUpdateHotkey(accelerator)}>
            {formatHotkeyLabel(accelerator, platform)}
          </button>
        ))}
      </div>
      {platform === "win32" ? <SettingsInlineMessage tone="info" role="status">
        <strong>{t("searchScopeSettings")}</strong>
        <span>{t("globalSearchWindowsScopeDesc")}</span>
        <button className={buttonSecondary} onClick={() => requestSettingsSection("settings-global-index")}>{t("globalSearchManage")}</button>
      </SettingsInlineMessage> : <>
      <SettingsRow label={t("searchScopeSettings")} description={t("searchScopeSettingsDesc")}>
        <SettingsSegmentedControl
          value={searchScopeMode}
          ariaLabel={t("searchScopeSettings")}
          options={[
            { value: "all", label: t("searchScopeAllIndexed") },
            { value: "current_scan", label: t("searchScopeCurrentScan") },
            { value: "custom_roots", label: t("searchScopeCustomRoots") }
          ]}
          onChange={onSearchScopeMode}
        />
      </SettingsRow>
      <span className={quietText}>{t("searchLocalIndexBoundary")}</span>
      {searchScopeMode === "custom_roots" ? (
        <div className="grid gap-2">
          {customSearchRoots.length ? <div className="flex justify-end"><button className={buttonSecondary} onClick={onAddSearchRoot}><FolderPlus size={15} /><span>{t("addSearchFolder")}</span></button></div> : null}
          {customSearchRoots.length ? customSearchRoots.map((root) => {
            const state = backgroundRootState(root);
            return (
              <div key={root.id} className={cn(compactInteractiveRow(), "px-3 py-2")}>
                <div className="grid min-w-0 gap-3 min-[841px]:grid-cols-[minmax(0,1fr)_auto] min-[841px]:items-center">
                  <div className="min-w-0 text-left">
                    <label htmlFor={`search-root-${root.id}`} className="block truncate text-sm font-medium text-[var(--zc-text-primary)]">{root.label}</label>
                    <span className="block truncate text-xs leading-5 text-[var(--zc-text-tertiary)]" title={root.path}>{compactPath(root.path, 72)}</span>
                  </div>
                  <div className="flex flex-wrap items-center justify-start gap-2 min-[841px]:justify-end">
                    <SettingsSwitchControl id={`search-root-${root.id}`} checked={root.enabled} label={root.enabled ? t("disableSearchFolder") : t("enableSearchFolder")} onChange={(next) => onSetSearchRootEnabled(root, next)} />
                    <button className={cn(buttonSecondary, "min-h-8 px-3 py-1.5 text-xs")} onClick={() => onIndexSearchRootNow(root)} disabled={state === "indexing" || state === "queued"}>
                      <Play size={14} />
                      <span>{state === "indexing" ? t("backgroundIndexingShort") : state === "queued" ? t("backgroundIndexQueuedShort") : t("indexNow")}</span>
                    </button>
                    <button className={buttonIconDanger} onClick={() => onRequestDelete({ kind: "search", root })} title={t("deleteSearchFolder")} aria-label={t("deleteSearchFolder")}><Trash2 size={14} /></button>
                  </div>
                </div>
              </div>
            );
          }) : <SettingsEmptyState title={t("searchScopeCustomRoots")} description={t("searchScopeCustomEmpty")} action={<button className={buttonSecondary} onClick={onAddSearchRoot}><FolderPlus size={15} /><span>{t("addSearchFolder")}</span></button>} />}
        </div>
      ) : null}
      </>}
      {platform !== "win32" && (isBackgroundIndexing || pendingBackgroundRoots.length > 0 || completedBackgroundRoots.length > 0 || failedBackgroundRoots.length > 0) ? (
        <SettingsInlineMessage tone={failedBackgroundRoots.length ? "warning" : isBackgroundIndexing ? "info" : "success"} role={failedBackgroundRoots.length ? "alert" : "status"}>
          <div className="flex flex-wrap items-center justify-between gap-2"><strong>{t("backgroundIndexingTitle")}</strong><span className="text-xs">{isBackgroundIndexing ? t("backgroundIndexingRunning") : t("backgroundIndexingIdle")}</span></div>
          {currentBackgroundRoot ? <span className={quietText}>{t("backgroundIndexingCurrent")}: {compactPath(currentBackgroundRoot, 76)}</span> : null}
          {pendingBackgroundRoots.length ? <span className={quietText}>{t("backgroundIndexingQueue")}: {pendingBackgroundRoots.length.toLocaleString()}</span> : null}
          {completedBackgroundRoots[0] ? <span className={quietText}>{t("backgroundIndexingCompleted")}: {compactPath(completedBackgroundRoots[0], 76)}</span> : null}
          {failedBackgroundRoots[0] ? <span className={quietText}>{t("backgroundIndexingFailed")}: {compactPath(failedBackgroundRoots[0].path, 76)}</span> : null}
        </SettingsInlineMessage>
      ) : null}
      {platform !== "win32" ? <span className={quietText}>{t("searchScopeDoesNotChangeLibrary")}</span> : null}
    </SettingsSection>
  );
}
