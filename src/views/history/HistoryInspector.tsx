import { useRef, useState } from "react";
import { ArrowLeft, Copy, ExternalLink, FileWarning, Move, RotateCcw, Trash2 } from "lucide-react";
import { tauriApi } from "../../api/tauriApi";
import { formatCount } from "../../i18n";
import type { CleanupRestorePreviewItem, CleanupTrashBatch, CleanupTrashItem, OperationLog, RecoveryAction } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { buttonGhost, buttonSecondary, cn, buttonDanger, buttonPrimary, inputSurface } from "../../utils/tw";
import { formatDisplayPath, localizedStableError } from "../../utils/viewHelpers";
import { ConfirmDialog, mutedText, rowSurface, SideSheet } from "../shared/ui";
import {
  cleanupRestoreEligibility,
  cleanupBatchRestorableCount,
  type CleanupPreviewState,
  historyTime,
  isNoOpLog,
  isRestorableCleanupTrashItem,
  restoreEligibility,
  type OperationHistoryBatch
} from "./historyModel";
import { operationDisplayName } from "./HistoryBatchList";

function formatDate(value: string, t: Translator) {
  const timestamp = historyTime(value);
  if (!timestamp) return t("historyTimeUnavailable");
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(timestamp));
}

function localizedRestoreMessage(message: string | null | undefined, t: Translator) {
  const normalized = (message ?? "").toLocaleLowerCase();
  if (!normalized) return "";
  if (normalized.startsWith("recovery_action_keep_both_completed:")) {
    return t("historyRecoveryKeepBothCompleted").replace("{path}", formatDisplayPath((message ?? "").slice("recovery_action_keep_both_completed:".length)));
  }
  if (normalized.startsWith("recovery_action_move_completed:")) {
    return t("historyRecoveryMoveCompleted").replace("{path}", formatDisplayPath((message ?? "").slice("recovery_action_move_completed:".length)));
  }
  if (normalized === "recovery_action_delete_completed") return t("historyRecoveryDeleteCompleted");
  if (normalized === "recovery_action_retry_cleanup_completed") return t("historyRecoveryRetryCleanupCompleted");
  const stable = localizedStableError(message, t);
  if (stable !== message) return stable;
  if (normalized.includes("target file already exists") || normalized.includes("original path already exists") || normalized.includes("already exists") || normalized.includes("原路径已有文件")) return t("restoreErrorTargetExists");
  if (normalized.includes("source file does not exist") || normalized.includes("safe trash path is missing") || normalized.includes("not found") || normalized.includes("不存在") || normalized.includes("缺失")) return t("restoreErrorSourceMissing");
  if (normalized.includes("permission") || normalized.includes("access denied") || normalized.includes("权限")) return t("restoreErrorPermission");
  if (normalized.includes("in use") || normalized.includes("occupied") || normalized.includes("被占用")) return t("restoreErrorOccupied");
  if (normalized.includes("no longer restorable") || normalized.includes("blocked") || normalized.includes("阻止")) return t("restoreErrorBlocked");
  if (normalized.includes("already restored") || normalized.includes("已经恢复")) return t("restoreErrorAlreadyRestored");
  if (normalized.includes("processing") || normalized.includes("处理中")) return t("restoreErrorProcessing");
  if (normalized.includes("canceled") || normalized.includes("cancelled") || normalized.includes("取消")) return t("restoreErrorCanceled");
  return t("restoreErrorGeneric");
}

function localizedCleanupMessage(message: string | null | undefined, t: Translator) {
  if (!message) return "";
  const stable = localizedStableError(message, t);
  if (stable !== message) return stable;
  return localizedRestoreMessage(message, t);
}

export function restoreEligibilityLabel(log: OperationLog, t: Translator) {
  const reason = restoreEligibility(log).reason;
  const key = `historyEligibility${reason.charAt(0).toUpperCase()}${reason.slice(1)}` as Parameters<Translator>[0];
  return t(key);
}

export function operationStatusLabel(log: OperationLog, t: Translator) {
  if (log.restore_status === "restored") return t("historyStatusRestored");
  if (log.restore_status === "canceled") return t("historyStatusRestoreCanceled");
  if (log.restore_status === "failed") return t("historyStatusRestoreFailed");
  return operationExecutionStatusLabel(log, t);
}

function operationExecutionStatusLabel(log: OperationLog, t: Translator) {
  if (log.status === "pending") return t("historyStatusPending");
  if (isNoOpLog(log)) return t("historyStatusNoOp");
  if (log.status === "manual_review") return t("historyStatusManualReview");
  if (log.status === "failed") return t("historyStatusFailed");
  if (log.status === "skipped") return t("historyStatusSkipped");
  return t("historyStatusSuccess");
}

function operationRestoreStatusLabel(log: OperationLog, t: Translator) {
  if (log.restore_status === "restored") return t("historyStatusRestored");
  if (log.restore_status === "canceled") return t("historyStatusRestoreCanceled");
  if (log.restore_status === "failed") return t("historyStatusRestoreFailed");
  if (log.restore_status === "pending") return t("historyEligibilityPending");
  if (log.restore_status === "unavailable") return t("historyStatusUnavailable");
  if (log.restore_status === "manual_review") return t("historyStatusManualReview");
  return t("historyStatusNotRestored");
}

function batchStateLabel(value: OperationHistoryBatch["executionState"] | OperationHistoryBatch["restoreState"], t: Translator) {
  if (value === "success") return t("historyStatusSuccess");
  if (value === "partial") return t("historyStatusPartial");
  if (value === "failed") return t("historyStatusFailed");
  if (value === "skipped") return t("historyStatusSkipped");
  if (value === "pending") return t("historyStatusPending");
  if (value === "no_op") return t("historyStatusNoOp");
  if (value === "canceled" || value === "restore_canceled") return t("historyStatusRestoreCanceled");
  if (value === "restored") return t("historyStatusRestored");
  if (value === "partially_restored") return t("historyStatusPartiallyRestored");
  if (value === "restorable") return t("historyStatusRestorable");
  if (value === "restore_failed") return t("historyStatusRestoreFailed");
  if (value === "not_restored") return t("historyStatusNotRestored");
  return t("historyStatusUnavailable");
}

export function operationTypeLabel(log: OperationLog, t: Translator) {
  if (isNoOpLog(log)) return t("historyStatusNoOp");
  if (log.operation_type === "move") return t("operationMove");
  if (log.operation_type === "rename") return t("operationRename");
  if (log.operation_type === "move_rename") return t("operationMoveRename");
  if (log.operation_type === "copy") return t("operationCopy");
  if (log.operation_type === "duplicate") return t("operationDuplicate");
  if (log.operation_type === "replace") return t("operationReplace");
  if (log.operation_type === "permanent_delete") return t("operationPermanentDelete");
  if (log.operation_type === "move_to_trash") return t("operationMoveToTrash");
  return t("historyOperationSourceUnknown");
}

function operationCurrentPath(log: OperationLog) {
  return log.restore_status === "restored"
    ? log.path_before || log.source_path || log.target_path
    : log.path_after || log.target_path || log.path_before || log.source_path;
}

function operationOriginalPath(log: OperationLog) {
  return log.path_before || log.source_path || log.path_after || log.target_path;
}

function suggestedRecoveryTarget(log: OperationLog) {
  const original = log.path_before || log.path_after || log.restore_claim_path || "";
  const separator = Math.max(original.lastIndexOf("/"), original.lastIndexOf("\\"));
  const parent = separator >= 0 ? original.slice(0, separator + 1) : "";
  const name = separator >= 0 ? original.slice(separator + 1) : original;
  const extensionIndex = name.lastIndexOf(".");
  const stem = extensionIndex > 0 ? name.slice(0, extensionIndex) : name;
  const extension = extensionIndex > 0 ? name.slice(extensionIndex) : "";
  return `${parent}${stem || "recovered-item"} (recovered)${extension}`;
}

function DetailRow({ label, value, title, path = false }: { label: string; value: string; title?: string; path?: boolean }) {
  return <div className="grid min-w-0 gap-0.5"><dt className="text-[11px] font-semibold text-[var(--zc-text-tertiary)]">{label}</dt><dd className={cn("min-w-0 text-xs text-[var(--muted)]", path ? "break-words font-mono leading-5 [overflow-wrap:anywhere]" : "truncate")} title={title ?? value}>{value || "-"}</dd></div>;
}

export function HistoryInspector({
  batch,
  selectedIds,
  onToggle,
  onBack,
  onRecoveryAction,
  t
}: {
  batch: OperationHistoryBatch | undefined;
  selectedIds: ReadonlySet<string>;
  onToggle: (log: OperationLog, checked: boolean) => void;
  onBack?: () => void;
  onRecoveryAction?: (log: OperationLog, action: RecoveryAction, targetPath?: string) => Promise<void>;
  t: Translator;
}) {
  const [technicalOpen, setTechnicalOpen] = useState<Set<string>>(new Set());
  const [revealError, setRevealError] = useState<Record<string, string>>({});
  const [recoveryTargetLog, setRecoveryTargetLog] = useState<OperationLog | null>(null);
  const [recoveryTarget, setRecoveryTarget] = useState("");
  const [deleteRecoveryLog, setDeleteRecoveryLog] = useState<OperationLog | null>(null);
  const [recoveryBusyId, setRecoveryBusyId] = useState("");
  const [recoveryError, setRecoveryError] = useState("");
  const recoveryTargetRef = useRef<HTMLInputElement | null>(null);
  if (!batch) return <div className="grid min-h-56 place-items-center text-sm text-[var(--muted)]">{t("historyNoSelection")}</div>;

  async function reveal(log: OperationLog, explicitPath?: string) {
    const path = explicitPath || operationCurrentPath(log);
    try {
      setRevealError((current) => ({ ...current, [log.id]: "" }));
      await tauriApi.revealInFolder(path);
    } catch {
      setRevealError((current) => ({ ...current, [log.id]: t("historyPathRevealFailed") }));
    }
  }

  async function revealClaim(log: OperationLog) {
    const path = log.restore_claim_path;
    if (!path) return;
    try {
      setRevealError((current) => ({ ...current, [log.id]: "" }));
      await tauriApi.revealInFolder(path);
    } catch {
      setRevealError((current) => ({ ...current, [log.id]: t("historyPathRevealFailed") }));
    }
  }

  function toggleTechnical(id: string) {
    setTechnicalOpen((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function revealSourceClaim(log: OperationLog) {
    const path = log.source_claim_path;
    if (!path) return;
    try {
      setRevealError((current) => ({ ...current, [log.id]: "" }));
      await tauriApi.revealInFolder(path);
    } catch {
      setRevealError((current) => ({ ...current, [log.id]: t("historyPathRevealFailed") }));
    }
  }

  async function runRecoveryAction(log: OperationLog, action: RecoveryAction, targetPath?: string) {
    if (!onRecoveryAction || recoveryBusyId) return;
    setRecoveryBusyId(log.id);
    setRecoveryError("");
    try {
      await onRecoveryAction(log, action, targetPath);
      setRecoveryTargetLog(null);
      setDeleteRecoveryLog(null);
    } catch (error) {
      setRecoveryError(error instanceof Error ? error.message : String(error));
    } finally {
      setRecoveryBusyId("");
    }
  }

  return (
    <section aria-labelledby="history-inspector-title" className="grid gap-3">
      <div className="flex items-start gap-3">
        {onBack && <button type="button" className={buttonSecondary} onClick={onBack} aria-label={t("historyInspectorBack")}><ArrowLeft size={16} /></button>}
        <div className="min-w-0 flex-1">
          <h2 id="history-inspector-title" className="text-base font-semibold">{t("historyInspector")}</h2>
          <div className="mt-2 grid grid-cols-2 gap-2 text-xs sm:grid-cols-4">
            <div className="grid gap-0.5"><span className="text-[var(--muted)]">{t("historySummaryOperations")}</span><strong className="tabular-nums">{batch.total}</strong></div>
            <div className="grid gap-0.5"><span className="text-[var(--muted)]">{t("historyOperationStatus")}</span><strong>{batchStateLabel(batch.executionState, t)}</strong></div>
            <div className="grid gap-0.5"><span className="text-[var(--muted)]">{t("historyRestoreStatus")}</span><strong>{batchStateLabel(batch.restoreState, t)}</strong></div>
            <div className="grid gap-0.5"><span className="text-[var(--muted)]">{t("historySummaryRestorable")}</span><strong className="tabular-nums">{batch.restorable}</strong></div>
          </div>
        </div>
      </div>
      <div className="grid gap-2" role="list" aria-label={t("historyInspector")}>
        {batch.logs.map((log) => {
          const eligibility = restoreEligibility(log);
          const eligible = eligibility.executable;
          const needsRestoreLocationReview = ["missingSource", "backendBlocked", "restoreFailed", "manualReview"].includes(eligibility.reason);
          const currentPath = operationCurrentPath(log);
          const originalPath = operationOriginalPath(log);
          const rawError = log.restore_error || log.error_message;
          const technical = technicalOpen.has(log.id);
          return (
            <div className={cn(rowSurface, "grid gap-2")} key={log.id} role="listitem">
              <div className="flex items-start gap-3">
                <input
                  type="checkbox"
                  aria-label={`${t("historySelectItem")}: ${operationDisplayName(log)}`}
                  checked={eligible && selectedIds.has(log.id)}
                  disabled={!eligible}
                  onChange={(event) => onToggle(log, event.currentTarget.checked)}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <strong className="truncate text-sm" title={`${operationDisplayName(log)} · ${formatDisplayPath(currentPath)}`}>{operationDisplayName(log)}</strong>
                    <span className="text-xs text-[var(--muted)]">{operationTypeLabel(log, t)}</span>
                    <span className="rounded-[var(--zc-radius-control)] border border-[var(--zc-border)] px-2 py-0.5 text-[11px] text-[var(--muted)]">{t("historyOperationStatus")}: {operationExecutionStatusLabel(log, t)}</span>
                    <span className="rounded-[var(--zc-radius-control)] border border-[var(--zc-border)] px-2 py-0.5 text-[11px] text-[var(--muted)]">{t("historyRestoreStatus")}: {operationRestoreStatusLabel(log, t)}</span>
                  </div>
                  <dl className="mt-2 grid gap-1.5 sm:grid-cols-2">
                    <DetailRow label={t("historyCreatedAt")} value={formatDate(log.created_at, t)} />
                    <DetailRow label={t("historyRestoreEligibility")} value={restoreEligibilityLabel(log, t)} />
                    <DetailRow path label={t("historyRestoreDestination")} value={formatDisplayPath(originalPath)} />
                    <DetailRow path label={t("historyCurrentPath")} value={formatDisplayPath(currentPath)} />
                  </dl>
                  {needsRestoreLocationReview && <p className="mt-2 rounded-[var(--zc-radius-control)] border border-[var(--zc-warning-border)] bg-[var(--zc-warning-soft)] px-3 py-2 text-xs leading-5 text-[var(--zc-warning-text)]">{t("historyRestoreOriginalUnavailable")}</p>}
                  <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                    <span className={cn("font-medium", eligible ? "text-[var(--zc-success-text)]" : "text-[var(--muted)]")}>{restoreEligibilityLabel(log, t)}</span>
                    {rawError && <button type="button" className="text-[var(--zc-primary)]" aria-expanded={technical} onClick={() => toggleTechnical(log.id)}>{technical ? t("historyRestoreHideTechnical") : t("historyRestoreShowTechnical")}</button>}
                  </div>
                  {revealError[log.id] && <p className="mt-1 text-xs text-[var(--zc-danger-text)]">{revealError[log.id]}</p>}
                  {rawError && <p className="mt-1 text-xs text-[var(--zc-danger-text)]">{localizedRestoreMessage(rawError, t)}</p>}
                  {technical && log.restore_claim_path && <div className="mt-2 flex min-w-0 items-center gap-2 rounded-[var(--zc-radius-control)] bg-[var(--zc-surface-subtle)] p-2 text-[11px] text-[var(--muted)]"><span className="shrink-0 font-semibold">{t("historyRestoreClaimPath")}:</span><button type="button" className="min-w-0 break-words text-left font-mono underline [overflow-wrap:anywhere]" title={formatDisplayPath(log.restore_claim_path)} onClick={() => void revealClaim(log)}>{formatDisplayPath(log.restore_claim_path)}</button></div>}
                  {technical && log.source_claim_path && <div className="mt-2 flex min-w-0 items-center gap-2 rounded-[var(--zc-radius-control)] bg-[var(--zc-surface-subtle)] p-2 text-[11px] text-[var(--muted)]"><span className="shrink-0 font-semibold">{t("historySourceClaimPath")}:</span><button type="button" className="min-w-0 break-words text-left font-mono underline [overflow-wrap:anywhere]" title={formatDisplayPath(log.source_claim_path)} onClick={() => void revealSourceClaim(log)}>{formatDisplayPath(log.source_claim_path)}</button></div>}
                  {technical && rawError && <pre className="mt-2 max-h-24 overflow-auto whitespace-pre-wrap break-words rounded-[var(--zc-radius-control)] bg-[var(--zc-surface-subtle)] p-2 text-[11px] text-[var(--muted)]">{rawError}</pre>}
                  {onRecoveryAction && log.status === "manual_review" && (log.restore_status === "manual_review" || log.operation_phase === "source_cleanup_pending") && (log.restore_claim_path || log.source_claim_path || log.operation_type === "replace") && <div className="mt-3 grid gap-2 rounded-[var(--zc-radius-control)] border border-[var(--zc-warning-border)] bg-[var(--zc-warning-soft)] p-3 text-xs text-[var(--zc-warning-text)]">
                    <div className="flex items-start gap-2"><ShieldAlertIcon /><div><strong className="block">{t("historyRecoveryActionsTitle")}</strong><p className="mt-1 leading-5">{t(log.operation_phase === "source_cleanup_pending" ? "historyRecoverySourceCleanupDesc" : "historyRecoveryActionsDesc")}</p></div></div>
                    <div className="flex flex-wrap gap-2">
                      {log.operation_phase === "source_cleanup_pending" && <>
                        <button type="button" className={buttonSecondary} onClick={() => void reveal(log, log.path_before || log.source_path)}><ExternalLink size={14} aria-hidden="true" />{t("historyRecoveryRevealSource")}</button>
                        <button type="button" className={buttonSecondary} onClick={() => void reveal(log, log.path_after || log.target_path)}><ExternalLink size={14} aria-hidden="true" />{t("historyRecoveryRevealTarget")}</button>
                      </>}
                      {log.operation_phase === "source_cleanup_pending" && log.source_claim_path && <button type="button" className={buttonSecondary} disabled={recoveryBusyId === log.id} onClick={() => void runRecoveryAction(log, "retry_cleanup")}><RotateCcw size={14} aria-hidden="true" />{recoveryBusyId === log.id ? t("historyRecoveryWorking") : t("historyRecoveryRetryCleanup")}</button>}
                      <button type="button" className={buttonSecondary} disabled={recoveryBusyId === log.id} onClick={() => void runRecoveryAction(log, "keep_both")}><Copy size={14} aria-hidden="true" />{recoveryBusyId === log.id ? t("historyRecoveryWorking") : t("historyRecoveryKeepBoth")}</button>
                      <button type="button" className={buttonSecondary} disabled={recoveryBusyId === log.id} onClick={() => { setRecoveryError(""); setRecoveryTargetLog(log); setRecoveryTarget(suggestedRecoveryTarget(log)); }}><Move size={14} aria-hidden="true" />{t("historyRecoveryMove")}</button>
                      <button type="button" className={buttonDanger} disabled={recoveryBusyId === log.id} onClick={() => { setRecoveryError(""); setDeleteRecoveryLog(log); }}><Trash2 size={14} aria-hidden="true" />{t("historyRecoveryDelete")}</button>
                    </div>
                    {recoveryError && recoveryBusyId === "" && <p className="text-[var(--zc-danger-text)]" role="alert">{localizedRestoreMessage(recoveryError, t)}</p>}
                  </div>}
                </div>
                <button type="button" className={buttonGhost} aria-label={`${t("historyOpenPath")}: ${operationDisplayName(log)}`} title={formatDisplayPath(currentPath)} onClick={() => void reveal(log)}>
                  <ExternalLink size={15} />
                </button>
              </div>
            </div>
          );
        })}
      </div>
      <SideSheet
        open={Boolean(recoveryTargetLog)}
        title={t("historyRecoveryMoveTitle")}
        description={t("historyRecoveryMoveDesc")}
        closeLabel={t("cancel")}
        onClose={() => { if (!recoveryBusyId) setRecoveryTargetLog(null); }}
        initialFocusRef={recoveryTargetRef}
        footer={<div className="flex flex-wrap justify-end gap-2"><button type="button" className={buttonSecondary} disabled={Boolean(recoveryBusyId)} onClick={() => setRecoveryTargetLog(null)}>{t("cancel")}</button><button type="button" className={buttonPrimary} disabled={!recoveryTarget.trim() || Boolean(recoveryBusyId)} onClick={() => recoveryTargetLog && void runRecoveryAction(recoveryTargetLog, "move", recoveryTarget.trim())}>{recoveryBusyId ? t("historyRecoveryWorking") : t("historyRecoveryMoveConfirm")}</button></div>}
      >
        <label htmlFor="history-recovery-target" className="grid gap-1 text-sm font-medium">{t("historyRecoveryTargetLabel")}</label>
        <input ref={recoveryTargetRef} id="history-recovery-target" value={recoveryTarget} onChange={(event) => setRecoveryTarget(event.currentTarget.value)} className={cn(inputSurface, "mt-2 w-full")} placeholder={t("historyRecoveryTargetPlaceholder")} autoComplete="off" />
        <p className={cn(mutedText, "mt-2")}>{t("historyRecoveryTargetHint")}</p>
        {recoveryError && recoveryTargetLog && <p className="mt-2 text-sm text-[var(--zc-danger-text)]" role="alert">{localizedRestoreMessage(recoveryError, t)}</p>}
      </SideSheet>
      <ConfirmDialog
        open={Boolean(deleteRecoveryLog)}
        tone="danger"
        title={t("historyRecoveryDeleteTitle")}
        description={t("historyRecoveryDeleteDesc")}
        confirmLabel={t("historyRecoveryDeleteConfirm")}
        cancelLabel={t("cancel")}
        isProcessing={Boolean(recoveryBusyId)}
        errorMessage={recoveryError ? localizedRestoreMessage(recoveryError, t) : undefined}
        onCancel={() => { if (!recoveryBusyId) setDeleteRecoveryLog(null); }}
        onConfirm={() => { if (deleteRecoveryLog) void runRecoveryAction(deleteRecoveryLog, "delete"); }}
      />
    </section>
  );
}

function ShieldAlertIcon() {
  return <FileWarning size={16} className="mt-0.5 shrink-0" aria-hidden="true" />;
}

function cleanupStatusLabel(item: CleanupTrashItem, preview: CleanupRestorePreviewItem | undefined, previewState: CleanupPreviewState, t: Translator) {
  if (previewState === "loading") return t("cleanupPreviewLoading");
  if (previewState === "failed") return t("cleanupPreviewFailed");
  if (previewState === "unavailable" || !preview) return t("cleanupPreviewUnavailable");
  const eligibility = cleanupRestoreEligibility(preview, previewState);
  if (eligibility.executable) return t("cleanupTrashMoved");
  if (eligibility.reason === "conflict") return t("historyCleanupConflict");
  if (eligibility.reason === "missing") return t("historyCleanupMissing");
  if (eligibility.reason === "failed") return t("historyCleanupFailed");
  if (eligibility.reason === "pending") return t("historyEligibilityPending");
  if (eligibility.reason === "manualReview") return t("historyCleanupManualReview");
  if (item.status === "restored") return t("restored");
  return t("historyStatusUnavailable");
}

export function CleanupInspector({
  batch,
  previewById,
  previewState,
  previewError,
  onRetry,
  onBack,
  selectedIds,
  onToggle,
  t
}: {
  batch: CleanupTrashBatch | undefined;
  previewById: ReadonlyMap<string, CleanupRestorePreviewItem>;
  previewState: CleanupPreviewState;
  previewError?: string;
  onRetry?: () => void;
  onBack?: () => void;
  selectedIds: ReadonlySet<string>;
  onToggle: (item: CleanupTrashItem, checked: boolean) => void;
  t: Translator;
}) {
  const [technicalOpen, setTechnicalOpen] = useState<Set<string>>(new Set());
  const [revealError, setRevealError] = useState<Record<string, string>>({});
  if (!batch) return <div className="grid min-h-40 place-items-center text-sm text-[var(--muted)]">{t("cleanupTrashEmpty")}</div>;

  async function reveal(item: CleanupTrashItem) {
    const path = item.status === "restored" ? item.originalPath : item.trashPath;
    try {
      setRevealError((current) => ({ ...current, [item.id]: "" }));
      await tauriApi.revealInFolder(path);
    } catch {
      setRevealError((current) => ({ ...current, [item.id]: t("historyPathRevealFailed") }));
    }
  }

  async function revealClaim(item: CleanupTrashItem) {
    const path = item.sourceClaimPath;
    if (!path) return;
    try {
      setRevealError((current) => ({ ...current, [item.id]: "" }));
      await tauriApi.revealInFolder(path);
    } catch {
      setRevealError((current) => ({ ...current, [item.id]: t("historyPathRevealFailed") }));
    }
  }

  function toggleTechnical(id: string) {
    setTechnicalOpen((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  return (
    <section className="grid gap-3" aria-labelledby="cleanup-inspector-title" data-cleanup-batch-id={batch.id}>
      <div>
        <div className="flex items-start gap-2">{onBack && <button type="button" className={buttonSecondary} onClick={onBack} aria-label={t("historyInspectorBack")}><ArrowLeft size={16} /></button>}<h2 id="cleanup-inspector-title" className="text-base font-semibold">{t("historyCleanupScope")}</h2></div>
        <p className={cn(mutedText, "mt-1")}>{t("historyCleanupRestoreDesc")}</p>
        {previewState !== "ready" && <div className={cn("mt-2 flex flex-wrap items-center gap-2 rounded-[var(--zc-radius-control)] border px-3 py-2 text-xs", previewState === "failed" ? "border-[var(--zc-danger-border)] text-[var(--zc-danger-text)]" : "border-[var(--zc-control-border)] text-[var(--muted)]")} role="status"><span>{previewState === "loading" ? t("cleanupPreviewLoading") : previewState === "failed" ? t("cleanupPreviewFailed") : t("cleanupPreviewUnavailable")}</span>{previewError && <button type="button" className="text-[var(--zc-primary)]" title={previewError} onClick={() => setTechnicalOpen((current) => new Set(current).add("__preview__"))}>{t("historyRestoreShowTechnical")}</button>}{onRetry && <button type="button" className={buttonSecondary} disabled={previewState === "loading"} onClick={onRetry}>{t("cleanupPreviewRetry")}</button>}{technicalOpen.has("__preview__") && previewError && <pre className="basis-full whitespace-pre-wrap break-words">{previewError}</pre>}</div>}
      </div>
      <div className="grid gap-2" role="list">
        {batch.items.map((item) => {
          const preview = previewById.get(item.id);
          const eligible = previewState === "ready" && Boolean(preview) && isRestorableCleanupTrashItem(item, preview, previewState);
          const currentPath = item.status === "restored" ? item.originalPath : item.trashPath;
          const technical = technicalOpen.has(item.id);
          // Successful ledger messages are informational, not Restore errors.
          // Current backend preview refusals take precedence over old journal text.
          const rawMessage = item.status === "restored" || previewState !== "ready" ? ""
            : preview?.blockingReason || (item.status === "moved" ? "" : item.message || "");
          return (
            <div key={item.id} className={cn(rowSurface, "grid gap-2")} role="listitem">
              <div className="flex items-start gap-3">
                <input
                  type="checkbox"
                    aria-label={`${t("historySelectItem")}: ${item.name}`}
                    checked={selectedIds.has(item.id)}
                    disabled={previewState !== "ready" || !preview}
                    onChange={(event) => onToggle(item, event.currentTarget.checked)}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2"><RotateCcw size={15} aria-hidden="true" /><strong className="truncate text-sm" title={item.name}>{item.name}</strong></div>
                  <dl className="mt-2 grid gap-1.5 sm:grid-cols-2">
                    <DetailRow label={t("historyCreatedAt")} value={formatDate(item.movedAt, t)} />
                    <DetailRow label={t("historyRestoreEligibility")} value={cleanupStatusLabel(item, preview, previewState, t)} />
                    <DetailRow path label={t("historyCleanupOriginalPath")} value={formatDisplayPath(item.originalPath)} />
                    <DetailRow path label={t("historyCleanupCurrentPath")} value={formatDisplayPath(currentPath)} />
                  </dl>
                  <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                    <span className={cn("font-medium", eligible ? "text-[var(--zc-success-text)]" : "text-[var(--muted)]")}>{cleanupStatusLabel(item, preview, previewState, t)}</span>
                    {rawMessage && <button type="button" className="text-[var(--zc-primary)]" aria-expanded={technical} onClick={() => toggleTechnical(item.id)}>{technical ? t("historyRestoreHideTechnical") : t("historyRestoreShowTechnical")}</button>}
                  </div>
                  {revealError[item.id] && <p className="mt-1 text-xs text-[var(--zc-danger-text)]">{revealError[item.id]}</p>}
                  {rawMessage && <p className="mt-1 text-xs text-[var(--zc-danger-text)]">{localizedCleanupMessage(rawMessage, t)}</p>}
                  {technical && item.sourceClaimPath && <div className="mt-2 flex min-w-0 items-center gap-2 rounded-[var(--zc-radius-control)] bg-[var(--zc-surface-subtle)] p-2 text-[11px] text-[var(--muted)]"><span className="shrink-0 font-semibold">{t("historyRestoreClaimPath")}:</span><button type="button" className="min-w-0 break-words text-left font-mono underline [overflow-wrap:anywhere]" title={formatDisplayPath(item.sourceClaimPath)} onClick={() => void revealClaim(item)}>{formatDisplayPath(item.sourceClaimPath)}</button></div>}
                  {technical && rawMessage && <pre className="mt-2 max-h-24 overflow-auto whitespace-pre-wrap break-words rounded-[var(--zc-radius-control)] bg-[var(--zc-surface-subtle)] p-2 text-[11px] text-[var(--muted)]">{rawMessage}</pre>}
                </div>
                <button type="button" className={buttonGhost} aria-label={`${t("historyOpenPath")}: ${item.name}`} title={formatDisplayPath(currentPath)} onClick={() => void reveal(item)}>
                  <ExternalLink size={15} />
                </button>
                {!eligible && item.status !== "restored" && <FileWarning size={16} className="text-[var(--muted)]" aria-hidden="true" />}
              </div>
            </div>
          );
        })}
      </div>
      <p className={cn(mutedText, "tabular-nums")}>{previewState === "ready" ? formatCount(t, cleanupBatchRestorableCount(batch, [...previewById.values()]), { zero: "historyBatchItemsZero", one: "historyBatchItemsOne", other: "historyBatchItemsOther" }) : t("cleanupPreviewUnavailable")}{previewState === "ready" && ` ${t("restorable")}`}</p>
    </section>
  );
}
