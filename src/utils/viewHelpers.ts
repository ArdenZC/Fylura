import type { AppSnapshot, FileQuery, FileRecord, LibraryScope, OperationPreview } from "../types/domain";
import type { Translator } from "../types/ui";
export { preferredLanguage, preferredTheme } from "./uiPreferences";
import { normalizeProposedFileNameExtension } from "./fileNaming";
import { DEFAULT_SEARCH_HOTKEY, formatHotkeyLabel } from "./hotkeys";

export function filterFiles(files: FileRecord[], query: FileQuery): FileRecord[] {
  const search = query.search?.toLowerCase().trim();
  const filtered = files.filter((file) => {
    if (search && !`${file.name} ${file.path} ${file.context}`.toLowerCase().includes(search)) return false;
    if (query.fileType && query.fileType !== "All" && file.file_type !== query.fileType) return false;
    if (query.purpose && query.purpose !== "All" && file.purpose !== query.purpose) return false;
    if (query.lifecycle && query.lifecycle !== "All" && file.lifecycle !== query.lifecycle) return false;
    if (query.riskLevel && query.riskLevel !== "All" && file.risk_level !== query.riskLevel) return false;
    if (query.onlyNeedsConfirmation && !file.requires_confirmation) return false;
    return true;
  });

  const sortBy = query.sortBy ?? "modified_at";
  const direction = query.sortDirection === "asc" ? 1 : -1;
  return [...filtered].sort((a, b) => {
    const left = a[sortBy];
    const right = b[sortBy];
    if (typeof left === "number" && typeof right === "number") return (left - right) * direction;
    return String(left).localeCompare(String(right)) * direction;
  });
}

export function splitDisplaySize(label: string) {
  const [value, ...unitParts] = label.split(" ");
  return {
    value: value || label,
    unit: unitParts.join(" ")
  };
}

export function compactPath(value: string | null | undefined, maxLength = 42) {
  if (!value) return "-";
  const displayValue = formatDisplayPath(value);
  if (displayValue.length <= maxLength) return displayValue;
  return `${displayValue.slice(0, Math.max(8, Math.floor(maxLength * 0.45)))}...${displayValue.slice(-Math.max(8, Math.floor(maxLength * 0.35)))}`;
}

export function formatDisplayPath(path: string, platform?: "windows" | "unix") {
  const shouldUseWindowsSeparators = platform
    ? platform === "windows"
    : /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("\\\\");
  return shouldUseWindowsSeparators ? path.replace(/[\\/]/g, "\\") : path.replace(/\\/g, "/");
}

export function libraryScopeLabel(scope: LibraryScope, allLabel: string, emptyLabel: string, maxLength = 64) {
  if (scope.kind === "all") return allLabel;
  if (!scope.roots.length) return emptyLabel;
  if (scope.roots.length === 1) return compactPath(scope.roots[0], maxLength);
  return `${compactPath(scope.roots[0], Math.max(32, maxLength - 10))} +${scope.roots.length - 1}`;
}

export function delay(ms: number) {
  return new Promise<void>((resolve) => window.setTimeout(resolve, ms));
}

export function groupOperationPreviews(previews: OperationPreview[], t: Translator) {
  const groups = new Map<string, { rawPath: string; displayName: string; displayPath: string; items: OperationPreview[]; subgroups: Map<string, { rawPath: string; displayName: string; displayPath: string; items: OperationPreview[] }> }>();
  for (const preview of previews) {
    const directory = pathDirLike(preview.target_path);
    const relativeParts = relativeOrganizeParts(directory);
    const firstSegment = relativeParts[0] ?? folderNameLike(directory);
    const logicalPath = isCanonicalPreviewMain(firstSegment);
    const mainKey = logicalPath ? canonicalPreviewMainKey(firstSegment) : `real:${normalizePathLike(directory)}`;
    const subgroupParts = logicalPath ? relativeParts.slice(1) : [];
    const subgroupKey = subgroupParts.length ? subgroupParts.join("/") : "__root__";
    const mainPath = logicalPath ? mainKey : directory;
    const mainDisplayName = logicalPath ? previewMainFolderLabel(mainKey, t) : prettyFolderName(folderNameLike(directory));
    const mainDisplayPath = logicalPath ? previewMainFolderLabel(mainKey, t) : formatDisplayPath(directory);
    const subgroupPath = subgroupKey === "__root__" ? directory : `${mainKey}/${subgroupKey}`;
    const subgroupDisplayName = subgroupKey === "__root__" ? t("previewRootFiles") : previewSubgroupLabel(subgroupKey, t);
    const subgroupDisplayPath = subgroupKey === "__root__"
      ? mainDisplayPath
      : `${mainDisplayPath} / ${previewSubgroupLabel(subgroupKey, t)}`;
    const group = groups.get(mainKey) ?? {
      rawPath: mainPath,
      displayName: mainDisplayName,
      displayPath: mainDisplayPath,
      items: [],
      subgroups: new Map<string, { rawPath: string; displayName: string; displayPath: string; items: OperationPreview[] }>()
    };
    group.items.push(preview);
    const subgroup = group.subgroups.get(subgroupKey) ?? { rawPath: subgroupPath, displayName: subgroupDisplayName, displayPath: subgroupDisplayPath, items: [] };
    subgroup.items.push(preview);
    group.subgroups.set(subgroupKey, subgroup);
    groups.set(mainKey, group);
  }
  return [...groups.entries()].map(([key, group]) => ({
    key,
    rawPath: group.rawPath,
    displayName: group.displayName,
    displayPath: group.displayPath,
    items: group.items,
    subgroups: [...group.subgroups.entries()].map(([subKey, subgroup]) => ({
      key: subKey,
      rawPath: subgroup.rawPath,
      displayName: subgroup.displayName,
      displayPath: subgroup.displayPath,
      items: subgroup.items
    }))
  }));
}

export function formatPreviewDisplayPath(path: string, t: Translator): string {
  const displayPath = formatDisplayPath(path);
  const tokens = displayPath.split(/([\\/]+)/);
  let insideLogicalPath = false;
  return tokens.map((token) => {
    if (!token || /^[\\/]+$/.test(token)) return token;
    if (isCanonicalPreviewMain(token)) {
      insideLogicalPath = true;
      return previewMainFolderLabel(token, t);
    }
    return insideLogicalPath ? previewSubgroupLabel(token, t) : token;
  }).join("");
}

export function relativeZenCanvasParts(directory: string): string[] {
  return relativeOrganizeParts(directory);
}

export function relativeOrganizeParts(directory: string): string[] {
  const parts = directory.replace(/\\/g, "/").split("/").filter(Boolean);
  const zenIndex = parts.findIndex((part) => part.toLowerCase() === "zencanvas");
  if (zenIndex >= 0) return parts.slice(zenIndex + 1);
  const canonicalIndex = parts.findIndex(isCanonicalPreviewMain);
  if (canonicalIndex >= 0) return parts.slice(canonicalIndex);
  return [folderNameLike(directory)];
}

export function canonicalPreviewMainKey(segment: string): string {
  if (isCanonicalPreviewMain(segment)) return segment;
  const normalized = segment.toLowerCase().replace(/^\d+_/, "");
  if (["career", "finance", "study", "work", "personal", "media", "project", "projects", "identity", "documents", "images", "videos", "audio"].includes(normalized)) {
    return "20_Areas";
  }
  if (normalized.includes("archive") || normalized.includes("reference")) return "40_Archive";
  if (
    normalized.includes("temporary") ||
    normalized.includes("temp") ||
    normalized.includes("installer") ||
    normalized.includes("download") ||
    normalized.includes("screenshot")
  ) {
    return "90_Temporary";
  }
  if (normalized.includes("inbox")) return "00_Inbox";
  return "20_Areas";
}

export function isCanonicalPreviewMain(segment: string): boolean {
  const normalized = segment.toLowerCase();
  return normalized.startsWith("00_") || normalized.startsWith("20_") || normalized.startsWith("40_") || normalized.startsWith("90_");
}

export function previewMainFolderLabel(key: string, t: Translator): string {
  const normalized = key.toLowerCase();
  if (normalized.startsWith("00_") || normalized.includes("inbox")) return t("previewInboxFolder");
  if (normalized.startsWith("20_") || normalized.includes("areas")) return t("previewAreasFolder");
  if (normalized.startsWith("40_") || normalized.includes("archive")) return t("previewArchiveFolder");
  if (normalized.startsWith("90_") || normalized.includes("temporary")) return t("previewTemporaryFolder");
  return prettyFolderName(key);
}

export function prettyFolderName(value: string): string {
  return value
    .split("/")
    .map((part) => part.replace(/^\d+_/, "").replace(/[_-]+/g, " "))
    .join(" / ");
}

export function defaultPlatformAccelerator(platform: NodeJS.Platform | "browser"): string {
  return formatHotkeyLabel(DEFAULT_SEARCH_HOTKEY, platform);
}

export function createOperationPreviews(files: FileRecord[]): OperationPreview[] {
  return files
    .filter((file) => ["Move", "Rename", "MoveAndRename", "Archive"].includes(file.suggested_action))
    .filter((file) => file.risk_level !== "Sensitive")
    .map((file) => {
      const proposedName = file.suggested_name || file.name;
      const normalizedName = normalizeProposedFileNameExtension(file.name, proposedName);
      const newName = normalizedName.name;
      const targetDirectory =
        file.suggested_target_path || (file.suggested_action === "Rename" ? file.directory : "");
      const targetPath = targetDirectory ? joinPathLike(targetDirectory, newName) : file.path;
      const isMove = Boolean(targetDirectory) && normalizePathLike(targetDirectory) !== normalizePathLike(file.directory);
      const isRename = newName !== file.name;
      const operationType: OperationPreview["operation_type"] =
        isMove && isRename ? "move_rename" : isMove ? "move" : "rename";
      const requiresConfirmation = file.requires_confirmation || file.confidence < 0.7 || normalizedName.error !== null;
      return {
        id: localId("op"),
        fileId: file.id,
        operation_type: operationType,
        source_path: file.path,
        target_path: targetPath,
        old_name: file.name,
        new_name: newName,
        status: "pending" as const,
        risk_level: file.risk_level,
        confidence: file.confidence,
        requires_confirmation: requiresConfirmation,
        suggested_action: file.suggested_action,
        is_duplicate: file.is_duplicate,
        reason: file.classification_reason,
        selected_by_default: !requiresConfirmation,
        is_executable: normalizedName.error === null,
        blocking_reason: normalizedName.error ? "Changing a file extension is not allowed during organization." : undefined,
        editable_new_name: normalizedName.error === null
      };
    })
    .filter((preview) => normalizePathLike(preview.source_path) !== normalizePathLike(preview.target_path));
}

export function applyPreviewNameOverride(preview: OperationPreview, name?: string): OperationPreview {
  const trimmed = name?.trim();
  if (!trimmed || trimmed === preview.new_name) return preview;
  const directory = pathDirLike(preview.target_path);
  return {
    ...preview,
    new_name: trimmed,
    target_path: joinPathLike(directory, trimmed),
    operation_type: normalizePathLike(directory) === normalizePathLike(pathDirLike(preview.source_path))
      ? "rename"
      : "move_rename"
  };
}

export function joinPathLike(directory: string, name: string): string {
  const separator = directory.includes("\\") ? "\\" : "/";
  return `${directory.replace(/[\\/]+$/, "")}${separator}${name}`;
}

export function pathDirLike(filePath: string): string {
  const normalized = filePath.replace(/[\\/]+$/, "");
  const index = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
  return index > 0 ? normalized.slice(0, index) : normalized;
}

export function folderNameLike(folderPath: string): string {
  const normalized = folderPath.replace(/[\\/]+$/, "");
  const index = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
  return index >= 0 ? normalized.slice(index + 1) || normalized : normalized;
}

export function normalizePathLike(value: string): string {
  const normalized = value.replace(/\\/g, "/").replace(/\/+$/, "");
  return isWindowsPathLike(normalized) ? normalized.toLowerCase() : normalized;
}

export function previewSubgroupLabel(value: string, t: Translator): string {
  const lifecycleKeys: Record<string, Parameters<Translator>[0]> = {
    inbox: "libraryLifecycleInbox", active: "libraryLifecycleActive", reference: "libraryLifecycleReference",
    archive: "libraryLifecycleArchive", disposable: "libraryLifecycleDisposable", duplicate: "libraryLifecycleDuplicate",
    sensitive: "libraryLifecycleSensitive", work: "libraryPurposeWork", downloads: "previewDownloadsFolder", download: "previewDownloadsFolder"
  };
  return value.split("/").map((part) => {
    const normalized = part.replace(/^\d+_/, "").toLowerCase();
    const key = lifecycleKeys[normalized];
    return key ? t(key) : prettyFolderName(part);
  }).join(" / ");
}

function isWindowsPathLike(value: string): boolean {
  return /^[a-z]:\//i.test(value) || value.startsWith("//");
}

export function fileBelongsToRoots(file: FileRecord, roots: string[]): boolean {
  const filePath = normalizePathLike(file.path);
  return roots.some((root) => {
    const normalizedRoot = normalizePathLike(root);
    return filePath === normalizedRoot || filePath.startsWith(`${normalizedRoot}/`);
  });
}

export function samePathLike(left: string, right: string): boolean {
  return normalizePathLike(left) === normalizePathLike(right);
}

export function sumUniqueDiskTotal(roots: AppSnapshot["scanRoots"]): number {
  const seen = new Set<string>();
  let total = 0;
  for (const root of roots) {
    const value = Number(root.disk_total_size ?? 0);
    if (!value) continue;
    const normalized = normalizePathLike(root.path);
    const volume = normalized.match(/^[a-z]:/)?.[0] ?? normalized.split("/")[0] ?? normalized;
    if (seen.has(volume)) continue;
    seen.add(volume);
    total += value;
  }
  return total;
}

export function prefersDarkScheme(): boolean {
  if (typeof window === "undefined") return false;
  return Boolean(window.matchMedia?.("(prefers-color-scheme: dark)")?.matches);
}

export function detectBrowserPlatform(): NodeJS.Platform | "browser" {
  if (typeof navigator === "undefined") return "browser";
  const platform = navigator.platform.toLowerCase();
  const userAgent = navigator.userAgent.toLowerCase();
  if (platform.includes("win") || userAgent.includes("windows")) return "win32";
  if (platform.includes("mac") || userAgent.includes("mac os")) return "darwin";
  if (platform.includes("linux") || userAgent.includes("linux")) return "linux";
  return "browser";
}

export function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function localizedStableError(error: unknown, t: Translator): string {
  const message = readableError(error);
  const settingsSaveFailure = /^settings_save_failure:([a-z_]+)$/.exec(message);
  if (settingsSaveFailure) {
    return `${t("settingsSaveFailed")} (${t("settingsFailureCode")}: ${settingsSaveFailure[1]})`;
  }
  if (message.startsWith("source_changed") || message.startsWith("source_identity_changed")) {
    return t("errorSourceIdentityChanged");
  }
  if (message.startsWith("source_claim_failed")) return t("errorSourceClaimFailed");
  if (message.startsWith("source_claim_mismatch")) return t("errorSourceClaimMismatch");
  if (message.startsWith("source_claim_rollback_failed")) return t("errorSourceClaimRollbackFailed");
  if (message.startsWith("source_claim_recovery_required")) return t("errorSourceClaimRecoveryRequired");
  if (message.startsWith("target_parent_identity_changed")) return t("errorTargetParentIdentityChanged");
  if (message.startsWith("atomic_source_binding_unsupported")) return t("errorAtomicSourceBindingUnsupported");
  if (message.startsWith("cross_volume_directory_move_unsupported")) return t("errorCrossVolumeDirectoryMoveUnsupported");
  if (message.startsWith("cross_volume_file_move_unsupported_on_macos")) return t("errorCrossVolumeFileMoveUnsupportedOnMacos");
  if (message.startsWith("unsupported_platform_linux")) return t("errorUnsupportedPlatformLinux");
  if (message.startsWith("macos_file_mutation_source_binding_unsupported")) return t("errorMacosFileMutationSourceBindingUnsupported");
  if (message.startsWith("staging_identity_changed")) return t("errorStagingIdentityChanged");
  if (message.startsWith("system_trash_source_binding_unsupported")) return t("errorSystemTrashSourceBindingUnsupported");
  if (message.startsWith("staging_handle_commit_unsupported")) return t("errorStagingHandleCommitUnsupported");
  if (message.startsWith("target_committed_durability_unknown")) return t("errorTargetCommittedDurabilityUnknown");
  if (message.startsWith("target_committed_identity_mismatch")) return t("errorTargetCommittedIdentityMismatch");
  if (message.includes("mac_source_retirement_pending")) return t("errorMacSourceRetirementPending");
  if (message.startsWith("target_committed_source_cleanup_pending")) return t("errorTargetCommittedSourceCleanupPending");
  if (message.startsWith("target_committed_source_delete_failed")) return t("errorTargetCommittedSourceDeleteFailed");
  if (message.startsWith("mac_provider_materialization_failed")) return t("errorMacProviderMaterializationFailed");
  if (message.startsWith("mac_provider_materialization_cancelled")) return t("errorMacProviderMaterializationCancelled");
  if (message.startsWith("mac_provider_materialization_required")) return t("errorMacProviderMaterializationRequired");
  if (message.startsWith("mac_provider_download_failed")) return t("errorMacProviderDownloadFailed");
  if (message.startsWith("mac_provider_coordination_failed")) return t("errorMacProviderCoordinationFailed");
  if (message.startsWith("mac_provider_url_changed")) return t("errorMacProviderUrlChanged");
  if (message.startsWith("mac_provider_identity_unavailable")) return t("errorMacProviderIdentityUnavailable");
  if (message.startsWith("mac_provider_item_unavailable")) return t("errorMacProviderItemUnavailable");
  if (message.startsWith("mac_provider_offline")) return t("errorMacProviderOffline");
  if (message.startsWith("mac_provider_permission_denied")) return t("errorMacProviderPermissionDenied");
  if (message.startsWith("mac_filesystem_capability_insufficient")) return t("errorMacFilesystemCapabilityInsufficient");
  if (message.startsWith("restore_pending_reconciliation")) return t("errorRestorePendingReconciliation");
  if (message.startsWith("claim_identity_mismatch")) return t("errorClaimIdentityMismatch");
  if (message.startsWith("manual_review_required")) return t("errorManualReviewRequired");
  if (message.startsWith("directory_manifest_name_encoding_failed")) return t("errorDirectoryManifestNameEncodingFailed");
  if (message.startsWith("copy_verification_failed")) return t("errorCopyVerificationFailed");
  if (message.startsWith("target_parent_durability_unknown")) return t("errorTargetParentDurabilityUnknown");
  if (message.startsWith("atomic_noreplace_unsupported")) {
    return t("errorAtomicNoReplaceUnsupported");
  }
  if (message.startsWith("credential_conflict")) {
    return t("errorCredentialConflict");
  }
  if (message.startsWith("credential_transaction_lock_poisoned")) {
    return t("errorCredentialTransactionUnavailable");
  }
  if (message.startsWith("settings_revision_conflict")) {
    return t("settingsRevisionConflict");
  }
  if (message.startsWith("watcher_retry_exhausted")) {
    return t("watcherRetryExhausted");
  }
  if (message.startsWith("cleanup_cancel_failed")) {
    return t("storageCleanupCancelFailed");
  }
  return message;
}

export function localId(prefix: string): string {
  return `${prefix}_${globalThis.crypto.randomUUID()}`;
}

export function nowIso(): string {
  return new Date().toISOString();
}
