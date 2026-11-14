import type { GlobalIndexSource, GlobalIndexStatus, RuntimeCapabilities } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { cn, focusVisibleState } from "../../../utils/tw";
import { quietText } from "../../shared/ui";
import type { GlobalIndexLoadState } from "../controllers/useSettingsGlobalIndexController";
import { SettingsControlGroup, SettingsRow, SettingsSection } from "../components/SettingsPrimitives";

export interface PlatformDiagnosticsSettingsSectionProps {
  t: Translator;
  capabilities: RuntimeCapabilities | null;
  globalIndexStatus: GlobalIndexStatus | null;
  globalIndexSources: GlobalIndexSource[];
  globalIndexLoadState: GlobalIndexLoadState;
  statusText: (status: string) => string;
}

function platformLabel(value: string | undefined, t: Translator) {
  if (value === "macos") return t("platformMacos");
  if (value === "windows") return t("platformWindows");
  if (value === "linux") return t("platformLinux");
  return t("platformUnknown");
}

function architectureLabel(value: string | undefined, t: Translator) {
  if (value === "aarch64") return t("architectureArm64");
  if (value === "x86_64") return t("architectureX8664");
  return value || t("platformUnknown");
}

function capabilityLabel(enabled: boolean | undefined, t: Translator) {
  return enabled ? t("platformCapabilityAvailable") : t("platformCapabilityUnavailable");
}

function filesystemLabel(sources: GlobalIndexSource[], t: Translator) {
  const filesystem = sources.find((source) => source.volume.filesystemType.trim())?.volume.filesystemType.trim();
  if (!filesystem) return t("platformUnknown");
  return filesystem.toLowerCase() === "apfs" ? "APFS" : filesystem;
}

function coverageLabel(status: string | undefined, loadState: GlobalIndexLoadState, hasSources: boolean, hasEnabledSource: boolean, t: Translator, statusText: (status: string) => string) {
  if (loadState !== "loaded") return t("platformUnknown");
  if (!hasSources) return t("platformCoverageNoSource");
  if (!hasEnabledSource) return t("platformCapabilityUnavailable");
  switch (status) {
    case "ready":
      return t("platformCoverageReady");
    case "partial":
      return t("platformCoveragePartial");
    case "permission_required":
      return t("platformCoveragePermissionRequired");
    case "spotlight_not_indexed":
      return t("platformCoverageSpotlightNotIndexed");
    default:
      return status ? statusText(status) : t("platformUnknown");
  }
}

export function PlatformDiagnosticsSettingsSection({
  t,
  capabilities,
  globalIndexStatus,
  globalIndexSources,
  globalIndexLoadState,
  statusText
}: PlatformDiagnosticsSettingsSectionProps) {
  const isMac = capabilities?.platform === "macos";
  const knownSources = globalIndexLoadState === "loaded" ? globalIndexSources : [];
  const hasSources = knownSources.length > 0;
  const hasEnabledSource = knownSources.some((source) => source.volume.enabled);
  const coverage = coverageLabel(globalIndexStatus?.status, globalIndexLoadState, hasSources, hasEnabledSource, t, statusText);
  const fsevents = !isMac
    ? t("platformNotApplicable")
    : globalIndexLoadState !== "loaded"
      ? t("platformUnknown")
    : !hasSources
      ? t("platformUnknown")
    : globalIndexStatus?.status === "fsevents_unavailable"
      ? t("platformCapabilityUnavailable")
      : globalIndexStatus
        ? t("platformCapabilityAvailable")
        : t("platformUnknown");

  return (
    <SettingsSection id="settings-platform-diagnostics" title={t("platformDiagnosticsTitle")} description={t("platformDiagnosticsDesc")}>
      <SettingsControlGroup title={t("platformIdentityTitle")} description={t("platformIdentityDesc")}>
        <SettingsRow label={t("platformDiagnosticsPlatform")}>
          <span className={quietText}>{platformLabel(capabilities?.platform, t)}</span>
        </SettingsRow>
        <SettingsRow label={t("platformDiagnosticsArchitecture")}>
          <span className={quietText}>{architectureLabel(capabilities?.architecture, t)}</span>
        </SettingsRow>
        <SettingsRow label={t("platformDiagnosticsMacosVersion")}>
          <span className={quietText}>{isMac ? capabilities?.macosVersion || t("platformUnknown") : t("platformNotApplicable")}</span>
        </SettingsRow>
        <SettingsRow label={t("platformDiagnosticsFilesystem")}>
          <span className={quietText}>{filesystemLabel(knownSources, t)}</span>
        </SettingsRow>
      </SettingsControlGroup>

      <SettingsControlGroup title={t("platformHealthTitle")} description={t("platformHealthDesc")}>
        <SettingsRow label={t("platformDiagnosticsSpotlight")}>
          <span className={quietText}>{isMac && globalIndexLoadState === "loaded" && globalIndexStatus ? statusText(globalIndexStatus.status) : isMac ? t("platformUnknown") : t("platformNotApplicable")}</span>
        </SettingsRow>
        <SettingsRow label={t("platformDiagnosticsFsevents")}>
          <span className={quietText}>{fsevents}</span>
        </SettingsRow>
        <SettingsRow label={t("platformDiagnosticsCoverage")}>
          <span className={quietText}>{coverage}</span>
        </SettingsRow>
      </SettingsControlGroup>

      <SettingsControlGroup title={t("platformSafetyTitle")} description={t("platformSafetyDesc")}>
        <SettingsRow label={t("platformDiagnosticsPackages")}><span className={quietText}>{capabilityLabel(capabilities?.packageMutationAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsICloud")}><span className={quietText}>{capabilityLabel(capabilities?.iCloudMutationAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsFileProvider")}><span className={quietText}>{capabilityLabel(capabilities?.fileProviderMutationAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsMutation")}><span className={quietText}>{capabilityLabel(capabilities?.fileMutationAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsCopy")}><span className={quietText}>{capabilityLabel(capabilities?.copyAvailable && capabilities?.duplicateAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsSameVolumeMutation")}><span className={quietText}>{capabilityLabel(capabilities?.sameVolumeMoveAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsCrossVolume")}><span className={quietText}>{capabilityLabel(capabilities?.crossVolumeMoveAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsRename")}><span className={quietText}>{capabilityLabel(capabilities?.renameAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsReplace")}><span className={quietText}>{capabilityLabel(capabilities?.replaceAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsSafeTrash")}><span className={quietText}>{capabilityLabel(capabilities?.safeTrashAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsRestore")}><span className={quietText}>{capabilityLabel(capabilities?.restoreAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsPermanentDelete")}><span className={quietText}>{capabilityLabel(capabilities?.permanentDeleteAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsSecureRemoval")}><span className={quietText}>{capabilityLabel(capabilities?.secureRemovalAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsExternalVolume")}><span className={quietText}>{capabilityLabel(capabilities?.externalVolumeMutationAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsNetworkVolume")}><span className={quietText}>{capabilityLabel(capabilities?.networkVolumeMutationAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsICloudMutation")}><span className={quietText}>{capabilityLabel(capabilities?.iCloudMutationAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsFileProviderMutation")}><span className={quietText}>{capabilityLabel(capabilities?.fileProviderMutationAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsLifecycle")}><span className={quietText}>{capabilityLabel(capabilities?.macosLifecycleAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsFinder")}><span className={quietText}>{capabilityLabel(capabilities?.macosFinderAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsQuickLookThumbnail")}><span className={quietText}>{capabilityLabel(capabilities?.macosQuickLookThumbnailAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsQuickLookPreview")}><span className={quietText}>{capabilityLabel(capabilities?.macosQuickLookPreviewAvailable, t)}</span></SettingsRow>
        <SettingsRow label={t("platformDiagnosticsActivityPolicy")}><span className={quietText}>{capabilityLabel(capabilities?.macosActivityPolicyAvailable, t)}</span></SettingsRow>
        {capabilities?.fileMutationUnavailableCode ? (
          <details className="rounded-[var(--zc-radius-control)] border border-[var(--zc-divider)] px-3 py-2" data-platform-diagnostics-technical-details>
            <summary className={cn("cursor-pointer text-xs font-medium text-[var(--zc-text-secondary)]", focusVisibleState)}>{t("platformDiagnosticsTechnicalDetails")}</summary>
            <p className="mt-2 text-xs leading-5 text-[var(--zc-text-tertiary)]">{t("platformDiagnosticsUnavailableReason")}</p>
            <code className="mt-2 block break-all rounded-[var(--zc-radius-control)] bg-[var(--zc-surface-subtle)] p-2 text-[11px] text-[var(--zc-text-tertiary)]">{capabilities.fileMutationUnavailableCode}</code>
          </details>
        ) : null}
      </SettingsControlGroup>
    </SettingsSection>
  );
}
