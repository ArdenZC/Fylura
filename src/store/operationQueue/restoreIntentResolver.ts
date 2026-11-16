import { makeTranslator } from "../../i18n";
import type {
  CleanupRestoreResult,
  OperationLog
} from "../../types/domain";
import { localId, localizedStableError, readableError } from "../../utils/viewHelpers";
import type { RestoreResultSummary } from "../../views/history/historyModel";
import { useAppStore } from "../useAppStore";

export function currentT() {
  return makeTranslator(useAppStore.getState().language);
}

export function createRestoreSessionId(source: "operation_logs" | "cleanup_trash") {
  return localId(`restore-${source}`);
}

export function localizedRestoreError(error: unknown, t: ReturnType<typeof currentT>) {
  const technical = readableError(error);
  const normalized = technical.toLocaleLowerCase();
  const stableMessage = localizedStableError(error, t);
  if (stableMessage !== technical) return { message: stableMessage, technical };
  let message = t("restoreErrorGeneric");
  if (normalized.includes("target file already exists") || normalized.includes("original path already exists") || normalized.includes("already exists") || normalized.includes("原路径已有文件")) {
    message = t("restoreErrorTargetExists");
  } else if (normalized.includes("source file does not exist") || normalized.includes("safe trash path is missing") || normalized.includes("not found") || normalized.includes("不存在") || normalized.includes("缺失")) {
    message = t("restoreErrorSourceMissing");
  } else if (normalized.includes("permission") || normalized.includes("access denied") || normalized.includes("权限")) {
    message = t("restoreErrorPermission");
  } else if (normalized.includes("in use") || normalized.includes("occupied") || normalized.includes("被占用")) {
    message = t("restoreErrorOccupied");
  } else if (normalized.includes("no longer restorable") || normalized.includes("blocked") || normalized.includes("阻止")) {
    message = t("restoreErrorBlocked");
  } else if (normalized.includes("already been restored") || normalized.includes("already restored") || normalized.includes("已经恢复")) {
    message = t("restoreErrorAlreadyRestored");
  } else if (normalized.includes("still being processed") || normalized.includes("processing") || normalized.includes("处理中")) {
    message = t("restoreErrorProcessing");
  } else if (normalized.includes("canceled") || normalized.includes("cancelled") || normalized.includes("取消")) {
    message = t("restoreErrorCanceled");
  } else if (normalized.startsWith("source_changed") || normalized.startsWith("source_identity_changed")) {
    message = t("errorSourceIdentityChanged");
  } else if (normalized.startsWith("source_claim_failed")) {
    message = t("errorSourceClaimFailed");
  } else if (normalized.startsWith("source_claim_mismatch")) {
    message = t("errorSourceClaimMismatch");
  } else if (normalized.startsWith("source_claim_rollback_failed")) {
    message = t("errorSourceClaimRollbackFailed");
  } else if (normalized.startsWith("source_claim_recovery_required")) {
    message = t("errorSourceClaimRecoveryRequired");
  } else if (normalized.startsWith("target_parent_identity_changed")) {
    message = t("errorTargetParentIdentityChanged");
  } else if (normalized.startsWith("atomic_source_binding_unsupported")) {
    message = t("errorAtomicSourceBindingUnsupported");
  } else if (normalized.startsWith("cross_volume_directory_move_unsupported")) {
    message = t("errorCrossVolumeDirectoryMoveUnsupported");
  } else if (normalized.startsWith("cross_volume_file_move_unsupported_on_macos")) {
    message = t("errorCrossVolumeFileMoveUnsupportedOnMacos");
  } else if (normalized.startsWith("unsupported_platform_linux")) {
    message = t("errorUnsupportedPlatformLinux");
  } else if (normalized.startsWith("macos_file_mutation_source_binding_unsupported")) {
    message = t("errorMacosFileMutationSourceBindingUnsupported");
  } else if (normalized.startsWith("staging_identity_changed")) {
    message = t("errorStagingIdentityChanged");
  } else if (normalized.startsWith("system_trash_source_binding_unsupported")) {
    message = t("errorSystemTrashSourceBindingUnsupported");
  } else if (normalized.startsWith("staging_handle_commit_unsupported")) {
    message = t("errorStagingHandleCommitUnsupported");
  } else if (normalized.startsWith("target_committed_durability_unknown")) {
    message = t("errorTargetCommittedDurabilityUnknown");
  } else if (normalized.startsWith("target_committed_identity_mismatch")) {
    message = t("errorTargetCommittedIdentityMismatch");
  } else if (normalized.startsWith("target_committed_source_cleanup_pending")) {
    message = t("errorTargetCommittedSourceCleanupPending");
  } else if (normalized.startsWith("target_committed_source_delete_failed")) {
    message = t("errorTargetCommittedSourceDeleteFailed");
  } else if (normalized.startsWith("directory_manifest_name_encoding_failed")) {
    message = t("errorDirectoryManifestNameEncodingFailed");
  } else if (normalized.startsWith("copy_verification_failed")) {
    message = t("errorCopyVerificationFailed");
  } else if (normalized.startsWith("target_parent_durability_unknown")) {
    message = t("errorTargetParentDurabilityUnknown");
  } else if (normalized.startsWith("atomic_noreplace_unsupported")) {
    message = t("errorAtomicNoReplaceUnsupported");
  }
  return { message, technical };
}

export function summarizeOperationRestore(logs: readonly OperationLog[], excluded: number): RestoreResultSummary {
  const missing = logs.filter((log) => log.restore_status === "unavailable" && /missing|缺失/i.test(log.restore_error ?? "")).length;
  return {
    requested: logs.length + excluded,
    restored: logs.filter((log) => log.restore_status === "restored").length,
    failed: logs.filter((log) => log.restore_status === "failed"
      || log.restore_status === "manual_review"
      || (log.restore_status === "unavailable" && !/missing|缺失/i.test(log.restore_error ?? ""))).length,
    skipped: logs.filter((log) => log.status === "skipped").length,
    canceled: logs.filter((log) => log.restore_status === "canceled").length,
    conflicts: 0,
    missing,
    excluded
  };
}

export function summarizeCleanupRestore(result: CleanupRestoreResult, excluded: number): RestoreResultSummary {
  return {
    requested: result.restored + result.conflicts + result.missing + result.failed + result.canceled + excluded,
    restored: result.restored,
    failed: result.failed,
    skipped: 0,
    canceled: result.canceled,
    conflicts: result.conflicts,
    missing: result.missing,
    excluded
  };
}
