import { Play } from "lucide-react";
import type { GlobalIndexSource, GlobalIndexStatus } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonSecondary, cn } from "../../../utils/tw";
import { compactPath } from "../../../utils/viewHelpers";
import { compactInteractiveRow, quietText } from "../../shared/ui";
import type { GlobalIndexLoadState } from "../controllers/useSettingsGlobalIndexController";
import { SettingsEmptyState, SettingsInlineMessage, SettingsSection, SettingsSwitchControl } from "../components/SettingsPrimitives";

export interface GlobalIndexSettingsSectionProps {
  t: Translator;
  status: GlobalIndexStatus | null;
  sources: GlobalIndexSource[];
  loadState: GlobalIndexLoadState;
  loadError: string | null;
  isLoading: boolean;
  isUpdating: boolean;
  statusText: (status: string) => string;
  providerStatusText: (status: string | null | undefined) => string | null;
  errorText: (error: string | null | undefined) => string | null;
  onAction: (action: () => Promise<void>, successMessage: string) => void;
}

function statusNeedsAttention(status: GlobalIndexStatus | null) {
  return Boolean(
    status?.status === "error"
      || status?.status === "permission_required"
      || status?.status === "authorization_required"
      || status?.status === "spotlight_not_indexed"
      || status?.status === "spotlight_external_not_indexed"
      || status?.status === "spotlight_unavailable"
      || status?.status === "fsevents_unavailable"
      || status?.lastError
      || status?.providerStatus?.includes("service_unavailable")
  );
}

export function GlobalIndexSettingsSection({
  t,
  status,
  sources,
  loadState,
  loadError,
  isLoading,
  isUpdating,
  statusText,
  providerStatusText,
  errorText,
  onAction
}: GlobalIndexSettingsSectionProps) {
  const hasSources = sources.length > 0;
  const isFullVolumeAuthorizationTarget = (source: GlobalIndexSource) =>
    source.volume.platform === "windows"
      && source.volume.driveKind === "fixed"
      && source.volume.filesystemType.trim().toLowerCase() === "ntfs";
  const sourceAuthorized = (source: GlobalIndexSource) =>
    source.volume.platform !== "windows" || source.volume.scopeAuthorizationState === "authorized";
  const hasEnabledSource = sources.some((source) => source.volume.enabled && sourceAuthorized(source));
  const hasUnconfirmedWindowsSource = sources.some((source) =>
    isFullVolumeAuthorizationTarget(source) && source.volume.scopeAuthorizationState !== "authorized"
  );
  const showLoading = isLoading || loadState === "loading";
  const sourceState = !hasSources
    ? "no_source"
    : !hasEnabledSource
      ? hasUnconfirmedWindowsSource ? "authorization_required" : "unavailable"
      : status?.status ?? "unknown";
  const attention = !hasEnabledSource || !status || statusNeedsAttention(status);
  const partial = Boolean(
    status?.status === "partial"
      || (status && !status.collectionComplete && status.status !== "indexing" && status.status !== "syncing")
      || (status && status.pendingVolumes > 0)
  );
  const sourceStatusText = (source: GlobalIndexSource) => {
    if (isFullVolumeAuthorizationTarget(source)) {
      if (source.volume.scopeAuthorizationState === "not_authorized") return t("globalIndexSourceDiscoveredNotEnabled");
      if (source.volume.scopeAuthorizationState === "legacy_confirmation_required") return t("globalIndexSourceLegacyConfirmationRequired");
      if (source.volume.scopeAuthorizationState === "authorized" && source.volume.enabled && source.volume.indexStatus === "discovered") {
        return t("globalIndexSourceAuthorizedWaiting");
      }
    }
    return statusText(source.volume.indexStatus);
  };
  const setSourceEnabled = (source: GlobalIndexSource, enabled: boolean) => {
    const needsAuthorization = isFullVolumeAuthorizationTarget(source)
      && source.volume.scopeAuthorizationState !== "authorized";
    if (enabled && needsAuthorization) {
      if (!window.confirm(t("globalIndexFullDiskConfirm"))) return;
      onAction(
        () => import("../../../api/tauriApi").then(({ tauriApi }) => tauriApi.authorizeGlobalIndexSource(source.volume.id, true)),
        t("globalIndexAuthorizationSaved")
      );
      return;
    }
    onAction(
      () => import("../../../api/tauriApi").then(({ tauriApi }) => tauriApi.setGlobalIndexSourceEnabled(source.volume.id, enabled)),
      t("settingsSavedInline")
    );
  };
  return (
    <SettingsSection id="settings-global-index" title={t("globalIndexTitle")} description={t("globalIndexDesc")} progressiveDisclosure>
      {loadState === "error" ? (
        <SettingsInlineMessage tone="warning" role="alert" status="load_failed">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <strong>{t("globalIndexStatus")}</strong>
            <span>{t("globalIndexLoadFailed")}</span>
          </div>
          <span className={quietText}>{loadError ?? t("globalIndexStatusUnknown")}</span>
        </SettingsInlineMessage>
      ) : showLoading ? (
        <SettingsEmptyState title={t("globalIndexLoading")} description={t("globalIndexLoadingDesc")} />
      ) : (
        <>
          {!hasSources ? (
            <SettingsInlineMessage tone="warning" role="status" status="no_source">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <strong>{t("globalIndexStatus")}</strong>
                <span>{t("globalIndexStatusUnavailable")}</span>
              </div>
              <span className={quietText}>{t("globalIndexNoSources")}</span>
              <span className={quietText}>{t("globalIndexNoSourcesDesc")}</span>
              {status ? (
                <span className={quietText}>
                  {t("globalIndexProcessed")}: {(status.processedEntries ?? status.totalEntries).toLocaleString()} · {status.collectionComplete ? t("globalIndexCollectionComplete") : t("globalIndexCollectionCollecting")}
                </span>
              ) : null}
              {status?.providerStatus ? <span className={quietText}>{t("globalIndexProvider")}: {providerStatusText(status.providerStatus)}</span> : null}
              {status?.lastError ? <span className={quietText}>{errorText(status.lastError)}</span> : null}
            </SettingsInlineMessage>
          ) : !hasEnabledSource ? (
            <SettingsInlineMessage tone="warning" role="status" status={hasUnconfirmedWindowsSource ? "authorization_required" : "unavailable"}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <strong>{t("globalIndexStatus")}</strong>
                <span>{hasUnconfirmedWindowsSource ? t("globalIndexStatusAuthorizationRequired") : t("globalIndexStatusUnavailable")}</span>
              </div>
              <span className={quietText}>{hasUnconfirmedWindowsSource ? t("globalIndexAuthorizationRequiredDesc") : t("globalIndexAllSourcesDisabled")}</span>
              {!hasUnconfirmedWindowsSource ? <span className={quietText}>{t("globalIndexAllSourcesDisabledDesc")}</span> : null}
            </SettingsInlineMessage>
          ) : status?.providerStatus?.includes("service_unavailable") ? (
            <SettingsInlineMessage tone="warning" role="alert">
              <strong>{t("globalIndexServiceUnavailable")}</strong>
              <span>{t("globalIndexServiceUnavailableDesc")}</span>
            </SettingsInlineMessage>
          ) : null}
          {hasSources && hasEnabledSource ? (
            <SettingsInlineMessage status={sourceState} tone={attention ? "warning" : partial ? "info" : "success"} role={attention ? "alert" : "status"}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <strong>{t("globalIndexStatus")}</strong>
                <span>{status ? statusText(status.status) : t("globalIndexStatusUnknown")}</span>
              </div>
              {status ? (
                <span className={quietText}>
                  {t("globalIndexProcessed")}: {(status.processedEntries ?? status.totalEntries).toLocaleString()} · {status.collectionComplete ? t("globalIndexCollectionComplete") : t("globalIndexCollectionCollecting")} · {t("globalIndexSources")}: {status.indexedVolumes.toLocaleString()}
                </span>
              ) : null}
              {status?.providerStatus ? <span className={quietText}>{t("globalIndexProvider")}: {providerStatusText(status.providerStatus)}</span> : null}
              {status?.lastError ? <span className={quietText}>{errorText(status.lastError)}</span> : null}
            </SettingsInlineMessage>
          ) : null}
          {hasSources && hasEnabledSource ? (
            <div className="flex flex-wrap gap-2">
              {status?.status === "indexing" || status?.status === "syncing" ? (
                <button className={buttonSecondary} onClick={() => onAction(() => import("../../../api/tauriApi").then(({ tauriApi }) => tauriApi.pauseGlobalIndex()), t("globalIndexPause"))} disabled={isUpdating}>
                  {t("globalIndexPause")}
                </button>
              ) : status?.status === "paused" ? (
                <button className={buttonSecondary} onClick={() => onAction(() => import("../../../api/tauriApi").then(({ tauriApi }) => tauriApi.resumeGlobalIndex()), t("globalIndexResume"))} disabled={isUpdating}>
                  {t("globalIndexResume")}
                </button>
              ) : (
                <button className={buttonSecondary} onClick={() => onAction(() => import("../../../api/tauriApi").then(({ tauriApi }) => tauriApi.startGlobalIndex()), t("globalIndexStart"))} disabled={isUpdating}>
                  {t("globalIndexStart")}
                </button>
              )}
            </div>
          ) : null}
          {sources.some((source) => source.volume.platform === "windows") ? <SettingsInlineMessage tone="info" role="status">
            {t("globalSearchWindowsScopeDesc")}
          </SettingsInlineMessage> : null}
          <div data-global-index-sources-state={loadState === "loaded" ? hasSources ? "available" : "no_source" : "unknown"} className="grid gap-2">
            {sources.map((source) => (
              <div key={source.volume.id} className={cn(compactInteractiveRow(), "px-3 py-2")}>
                <div className="grid min-w-0 gap-3 min-[841px]:grid-cols-[minmax(0,1fr)_auto] min-[841px]:items-center">
                  <div className="min-w-0 text-left">
                    <strong className="block truncate text-sm font-medium text-[var(--zc-text-primary)]">{source.volume.displayName}</strong>
                    <span className="block truncate text-xs leading-5 text-[var(--zc-text-tertiary)]" title={source.volume.mountPath}>
                      {compactPath(source.volume.mountPath, 72)} · {sourceStatusText(source)} · {source.volume.entryCount.toLocaleString()}
                    </span>
                  </div>
                  {loadState === "loaded" ? (
                    <div className="flex flex-wrap items-center justify-start gap-2 min-[841px]:justify-end">
                      <SettingsSwitchControl
                        id={`global-index-source-${source.volume.id}`}
                        checked={source.volume.enabled && (sourceAuthorized(source) || !isFullVolumeAuthorizationTarget(source))}
                        label={source.volume.enabled && (sourceAuthorized(source) || !isFullVolumeAuthorizationTarget(source)) ? t("globalIndexEnabled") : isFullVolumeAuthorizationTarget(source) && source.volume.scopeAuthorizationState !== "authorized" ? t("globalIndexAuthorizationRequiredDesc") : t("globalIndexDisabled")}
                        disabled={isUpdating || (source.volume.platform === "windows" && !isFullVolumeAuthorizationTarget(source) && !source.volume.enabled)}
                        onChange={(enabled) => setSourceEnabled(source, enabled)}
                      />
                      <button className={cn(buttonSecondary, "min-h-8 px-3 py-1.5 text-xs")} onClick={() => onAction(() => import("../../../api/tauriApi").then(({ tauriApi }) => tauriApi.rebuildGlobalIndexSource(source.volume.id)), t("globalIndexRebuild"))} disabled={isUpdating || !source.canRebuild || !sourceAuthorized(source)}>
                        <Play size={14} />
                        <span>{t("globalIndexRebuild")}</span>
                      </button>
                    </div>
                  ) : null}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
    </SettingsSection>
  );
}
