import { FolderPlus, Play, Trash2 } from "lucide-react";
import type { WatcherReconciliationStatus } from "../../../api/tauriApi";
import type { FolderNamingLanguage, OrganizeRootMode, ScanRootSetting } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonIconDanger, buttonSecondary, cn } from "../../../utils/tw";
import { compactPath, normalizePathLike } from "../../../utils/viewHelpers";
import { deriveWatcherPresentation } from "../../../utils/watcherPresentation";
import { compactInteractiveRow, quietText } from "../../shared/ui";
import { SettingsControlGroup, SettingsEmptyState, SettingsRow, SettingsSection, SettingsSelect, SettingsSwitchControl, settingsField } from "../components/SettingsPrimitives";
import type { FolderDeleteConfirmState } from "./settingsSectionTypes";

export interface FileSourcesSettingsSectionProps {
  t: Translator;
  defaultScanFolders: ScanRootSetting[];
  watcherRootStatuses: Record<string, WatcherReconciliationStatus>;
  organizeRootMode: OrganizeRootMode;
  organizeRootPath: string | null;
  onAddScanFolder: () => void;
  onSetScanRootEnabled: (root: ScanRootSetting, enabled: boolean) => void;
  onScanRootNow: (root: ScanRootSetting) => void;
  onRequestDelete: (state: FolderDeleteConfirmState) => void;
  onOrganizeRootMode: (value: OrganizeRootMode) => void;
  onOrganizeRootPath: (value: string) => void;
  onChooseOrganizeRootPath: () => void;
}

function watcherStatusForSetting(root: ScanRootSetting, statuses: Record<string, WatcherReconciliationStatus>) {
  const exact = statuses[root.id];
  if (exact) return exact;
  const targetPath = normalizePathLike(root.path);
  return Object.values(statuses).find((status) => normalizePathLike(status.path) === targetPath);
}

export function FileSourcesSettingsSection({
  t,
  defaultScanFolders,
  watcherRootStatuses,
  organizeRootMode,
  organizeRootPath,
  onAddScanFolder,
  onSetScanRootEnabled,
  onScanRootNow,
  onRequestDelete,
  onOrganizeRootMode,
  onOrganizeRootPath,
  onChooseOrganizeRootPath
}: FileSourcesSettingsSectionProps) {
  return (
    <SettingsSection id="settings-files-scan" title={t("settingsFilesScan")} description={t("settingsFilesScanDesc")}>
      <SettingsControlGroup title={t("settingsScanRoots")} description={t("settingsScanRootsDesc")}>
        {defaultScanFolders.length ? (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <span className={quietText}>{t("defaultScanFoldersRestartHint")}</span>
            <button className={buttonSecondary} onClick={onAddScanFolder}>
              <FolderPlus size={15} />
              <span>{t("addScanFolder")}</span>
            </button>
          </div>
        ) : null}
        <div className="grid gap-2">
          {defaultScanFolders.length ? defaultScanFolders.map((root) => {
            const watcherStatus = watcherStatusForSetting(root, watcherRootStatuses);
            const watcherPresentation = deriveWatcherPresentation(watcherStatus);
            return (
              <div key={root.id} data-settings-item className={cn(compactInteractiveRow(), "px-3 py-2")}>
                <div className="grid min-w-0 gap-3 min-[841px]:grid-cols-[minmax(0,1fr)_auto] min-[841px]:items-center">
                  <div className="min-w-0 text-left">
                    <label htmlFor={`scan-root-${root.id}`} className="block truncate text-sm font-medium text-[var(--zc-text-primary)]">{root.label}</label>
                    <span className="block truncate text-xs leading-5 text-[var(--zc-text-tertiary)]" title={root.path}>{compactPath(root.path, 72)}</span>
                    <div className="mt-1 flex items-center gap-2 text-xs">
                      <span className="rounded-[var(--zc-radius-control)] border border-[var(--zc-border)] px-2 py-0.5 text-[var(--zc-text-secondary)]">{t(watcherPresentation.labelKey)}</span>
                    </div>
                  </div>
                  <div className="flex flex-wrap items-center justify-start gap-2 min-[841px]:justify-end">
                    <SettingsSwitchControl id={`scan-root-${root.id}`} checked={root.enabled} label={root.enabled ? t("disableScanFolder") : t("enableScanFolder")} onChange={(next) => onSetScanRootEnabled(root, next)} />
                    <button className={cn(buttonSecondary, "min-h-8 px-3 py-1.5 text-xs")} onClick={() => onScanRootNow(root)} title={t("scanNow")}>
                      <Play size={14} />
                      <span>{t("scanNow")}</span>
                    </button>
                    <button className={buttonIconDanger} onClick={() => onRequestDelete({ kind: "scan", root })} title={t("deleteScanFolder")} aria-label={t("deleteScanFolder")}>
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>
              </div>
            );
          }) : (
            <SettingsEmptyState title={t("defaultScanFolders")} description={t("noDefaultScanFolders")} action={(
              <button className={buttonSecondary} onClick={onAddScanFolder}>
                <FolderPlus size={15} />
                <span>{t("addScanFolder")}</span>
              </button>
            )} />
          )}
        </div>
      </SettingsControlGroup>

      <SettingsControlGroup title={t("settingsOrganizeRoot")} description={t("settingsOrganizeRootDesc")}>
        <SettingsSelect
          id="settings-organize-root-mode"
          label={t("settingsOrganizeRoot")}
          description={`${organizeRootMode === "current_folder" ? t("organizeRootCurrentDesc") : organizeRootMode === "zen_canvas_folder" ? t("organizeRootZenCanvasDesc") : t("organizeRootCustomDesc")} ${t("organizePreviewStillRequired")}`}
          value={organizeRootMode}
          options={[
            { value: "current_folder" as const, label: t("organizeRootCurrentFolder") },
            { value: "zen_canvas_folder" as const, label: t("organizeRootZenCanvasFolder") },
            { value: "custom_root" as const, label: t("organizeRootCustomRoot") }
          ]}
          onChange={onOrganizeRootMode}
        />
        {organizeRootMode === "custom_root" ? (
          <SettingsRow label={t("organizeRootCustomRoot")} description={t("organizeRootCustomDesc")}>
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <input className={cn(settingsField, "min-w-0 flex-1")} value={organizeRootPath ?? ""} onChange={(event) => onOrganizeRootPath(event.target.value)} placeholder={t("organizeRootPathPlaceholder")} aria-label={t("organizeRootCustomRoot")} />
              <button className={buttonSecondary} onClick={onChooseOrganizeRootPath}>
                <FolderPlus size={15} />
                <span>{t("chooseFolders")}</span>
              </button>
            </div>
          </SettingsRow>
        ) : null}
      </SettingsControlGroup>
    </SettingsSection>
  );
}
