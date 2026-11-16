import type {
  CleanupRestorePreviewItem,
  CleanupTrashBatch,
  CleanupTrashItem,
  OperationLog
} from "../../types/domain";

/**
 * Restore eligibility is deliberately ordered to mirror the Rust restore
 * command. The first matching reason is the only reason shown to users.
 * Keeping this in one pure module prevents the list, inspector and store from
 * disagreeing about what the backend can accept.
 */
export type RestoreEligibilityReason =
  | "restorable"
  | "alreadyRestored"
  | "unsupportedOperation"
  | "noOp"
  | "failedOperation"
  | "manualReview"
  | "pending"
  | "backendBlocked"
  | "unavailable"
  | "missingSource"
  | "restoreFailed"
  | "canceled";

export type CleanupRestoreEligibilityReason =
  | "restorable"
  | "alreadyRestored"
  | "conflict"
  | "missing"
  | "failed"
  | "pending"
  | "manualReview"
  | "unavailable";

export type CleanupPreviewState = "loading" | "ready" | "failed" | "unavailable";

export interface CleanupPreviewAuthority {
  state: CleanupPreviewState;
  preview?: CleanupRestorePreviewItem;
  error?: string | null;
}

export interface RestoreEligibility {
  executable: boolean;
  reason: RestoreEligibilityReason;
}

export function restoreEligibility(log: OperationLog): RestoreEligibility {
  if (isNoOpLog(log)) return { executable: false, reason: "noOp" };
  if (log.operation_type === "move_to_trash" && log.path_after === "Recycle Bin") return { executable: false, reason: "unsupportedOperation" };
  if (log.operation_type === "permanent_delete") return { executable: false, reason: "unsupportedOperation" };
  if (log.status === "manual_review" || log.restore_status === "manual_review") return { executable: false, reason: "manualReview" };
  if (log.status !== "success") return { executable: false, reason: "failedOperation" };
  if (log.restore_status === "restored") return { executable: false, reason: "alreadyRestored" };
  if (log.restore_status === "pending") return { executable: false, reason: "pending" };
  if (log.restore_status === "unavailable") return { executable: false, reason: "unavailable" };
  if (log.restore_status === "failed") return { executable: false, reason: "restoreFailed" };
  if (log.restore_status === "canceled") return { executable: false, reason: "canceled" };
  if (!log.can_restore) return { executable: false, reason: "backendBlocked" };
  if (!log.path_before || !log.path_after) return { executable: false, reason: "missingSource" };
  return { executable: true, reason: "restorable" };
}

export function isRestorableLog(log: OperationLog) {
  return restoreEligibility(log).executable;
}

/**
 * No-op is a backend operation classification, not a synonym for a skipped
 * record. Only an explicit operation type can produce the no-op presentation.
 */
export function isNoOpLog(log: OperationLog) {
  const normalized = log.operation_type.trim().toLocaleLowerCase().replace(/[\s-]+/g, "_");
  return normalized === "no_op" || normalized === "noop" || normalized === "unchanged";
}

function uniqueIds(selectedIds: ReadonlySet<string> | readonly string[]) {
  return [...new Set(selectedIds instanceof Set ? [...selectedIds] : selectedIds)];
}

function increment(counts: Record<string, number>, key: string) {
  counts[key] = (counts[key] ?? 0) + 1;
}

function countsEqual(left: Record<string, number>, right: Record<string, number>) {
  const keys = new Set([...Object.keys(left), ...Object.keys(right)]);
  return [...keys].every((key) => (left[key] ?? 0) === (right[key] ?? 0));
}

function operationFingerprint(logs: readonly OperationLog[], ids: readonly string[]) {
  const byId = new Map(logs.map((log) => [log.id, log]));
  return ids
    .map((id) => {
      const log = byId.get(id);
      if (!log) return `${id}|missing`;
      return [
        id,
        log.batch_id,
        log.operation_type,
        log.status,
        log.restore_status,
        log.can_restore,
        log.path_before,
        log.path_after,
        log.error_message ?? "",
        log.restore_error ?? ""
      ].join("\u001f");
    })
    .join("\u001e");
}

export interface RestoreSelectionResolution {
  selectedIds: string[];
  selectedCount: number;
  executable: OperationLog[];
  executableIds: string[];
  executableCount: number;
  excludedCount: number;
  missingIds: string[];
  reasonCounts: Record<string, number>;
  fingerprint: string;
}

export function resolveOperationRestoreSelection(
  logs: readonly OperationLog[],
  selectedIds: ReadonlySet<string> | readonly string[]
): RestoreSelectionResolution {
  const ids = uniqueIds(selectedIds);
  const byId = new Map(logs.map((log) => [log.id, log]));
  const executable: OperationLog[] = [];
  const reasonCounts: Record<string, number> = {};
  const missingIds: string[] = [];
  for (const id of ids) {
    const log = byId.get(id);
    if (!log) {
      missingIds.push(id);
      increment(reasonCounts, "unavailable");
      continue;
    }
    const eligibility = restoreEligibility(log);
    if (eligibility.executable) executable.push(log);
    else increment(reasonCounts, eligibility.reason);
  }
  return {
    selectedIds: ids,
    selectedCount: ids.length,
    executable,
    executableIds: executable.map((log) => log.id),
    executableCount: executable.length,
    excludedCount: ids.length - executable.length,
    missingIds,
    reasonCounts,
    fingerprint: operationFingerprint(logs, ids)
  };
}

export function selectionForOperationBatch(
  current: ReadonlySet<string>,
  logs: readonly OperationLog[],
  select: boolean
) {
  const next = new Set(current);
  for (const log of logs) {
    if (select) {
      if (isRestorableLog(log)) next.add(log.id);
    } else {
      next.delete(log.id);
    }
  }
  return next;
}

/**
 * Reconcile view-local selection against the latest operation-log authority.
 * A refresh can change a previously restorable record into a blocked or
 * already-restored record; those ids must leave the selection immediately so
 * the sticky restore action cannot describe stale intent.
 */
export function reconcileRestorableOperationSelection(
  logs: readonly OperationLog[],
  selectedIds: ReadonlySet<string> | readonly string[]
) {
  const restorableIds = new Set(logs.filter(isRestorableLog).map((log) => log.id));
  return new Set(uniqueIds(selectedIds).filter((id) => restorableIds.has(id)));
}

function cleanupReasonFromBlockingReason(reason: string | null | undefined): CleanupRestoreEligibilityReason {
  const normalized = (reason ?? "").toLocaleLowerCase();
  if (normalized.includes("restored") || normalized.includes("已恢复")) return "alreadyRestored";
  if (normalized.includes("conflict") || normalized.includes("exists") || normalized.includes("原路径") || normalized.includes("original")) return "conflict";
  if (normalized.includes("missing") || normalized.includes("not found") || normalized.includes("不存在") || normalized.includes("缺失")) return "missing";
  if (normalized.includes("manual_review") || normalized.includes("manual review") || normalized.includes("人工复核") || normalized.includes("identity")) return "manualReview";
  if (normalized.includes("pending") || normalized.includes("processing") || normalized.includes("处理中")) return "pending";
  if (normalized.includes("failed") || normalized.includes("error") || normalized.includes("失败")) return "failed";
  return "unavailable";
}

export function cleanupRestoreEligibility(
  preview?: CleanupRestorePreviewItem,
  state: CleanupPreviewState = preview ? "ready" : "unavailable"
): { executable: boolean; reason: CleanupRestoreEligibilityReason } {
  if (state !== "ready" || !preview) return { executable: false, reason: "unavailable" };
  if (preview.canRestore) return { executable: true, reason: "restorable" };
  return { executable: false, reason: cleanupReasonFromBlockingReason(preview.blockingReason) };
}

export function isRestorableCleanupTrashItem(
  item: CleanupTrashItem,
  preview?: CleanupRestorePreviewItem,
  state: CleanupPreviewState = preview ? "ready" : "unavailable"
) {
  void item;
  return cleanupRestoreEligibility(preview, state).executable;
}

function cleanupFingerprint(
  items: readonly CleanupTrashItem[],
  ids: readonly string[],
  authorities?: ReadonlyMap<string, CleanupPreviewAuthority>
) {
  const byId = new Map(items.map((item) => [item.id, item]));
  return ids
    .map((id) => {
      const item = byId.get(id);
      if (!item) return `${id}|missing`;
      const authority = authorities?.get(id);
      const preview = authority?.preview ?? ("canRestore" in item && "blockingReason" in item ? item as CleanupRestorePreviewItem : undefined);
      return [
        id,
        item.batchId,
        item.status,
        authority?.state ?? (preview ? "ready" : "unavailable"),
        preview?.canRestore ?? "unknown",
        preview?.blockingReason ?? "",
        item.originalPath,
        item.trashPath,
        item.message ?? ""
      ].join("\u001f");
    })
    .join("\u001e");
}

export interface CleanupSelectionResolution {
  selectedIds: string[];
  selectedCount: number;
  executable: CleanupRestorePreviewItem[];
  executableIds: string[];
  executableCount: number;
  excludedCount: number;
  missingIds: string[];
  reasonCounts: Record<string, number>;
  fingerprint: string;
}

export function resolveCleanupRestoreSelection(
  items: readonly CleanupTrashItem[],
  selectedIds: ReadonlySet<string> | readonly string[],
  authorities?: ReadonlyMap<string, CleanupPreviewAuthority>
): CleanupSelectionResolution {
  const ids = uniqueIds(selectedIds);
  const byId = new Map(items.map((item) => [item.id, item]));
  const executable: CleanupRestorePreviewItem[] = [];
  const reasonCounts: Record<string, number> = {};
  const missingIds: string[] = [];
  for (const id of ids) {
    const item = byId.get(id);
    if (!item) {
      missingIds.push(id);
      increment(reasonCounts, "unavailable");
      continue;
    }
    const embeddedPreview = "canRestore" in item && "blockingReason" in item ? item as CleanupRestorePreviewItem : undefined;
    const authority = authorities?.get(id);
    const preview = authority?.preview ?? embeddedPreview;
    const state = authority?.state ?? (preview ? "ready" : "unavailable");
    const eligibility = cleanupRestoreEligibility(preview, state);
    if (eligibility.executable && preview) executable.push(preview);
    else increment(reasonCounts, eligibility.reason);
  }
  return {
    selectedIds: ids,
    selectedCount: ids.length,
    executable,
    executableIds: executable.map((item) => item.id),
    executableCount: executable.length,
    excludedCount: ids.length - executable.length,
    missingIds,
    reasonCounts,
    fingerprint: cleanupFingerprint(items, ids, authorities)
  };
}

export interface RestoreExecutionIntent {
  sessionId: string;
  source: "operation_logs" | "cleanup_trash";
  selectedIds: Set<string>;
  allowedIds: Set<string>;
  selectedCount: number;
  executableCount: number;
  excludedCount: number;
  reasonCounts: Record<string, number>;
  createdAt: number;
  revision: number;
  authorityFingerprint: string;
  batchIds?: Set<string>;
}

export type RestoreResolution = RestoreSelectionResolution | CleanupSelectionResolution;

export interface RestoreResultSummary {
  requested: number;
  restored: number;
  failed: number;
  skipped: number;
  canceled: number;
  conflicts: number;
  missing: number;
  excluded: number;
}

export function createRestoreExecutionIntent(
  source: RestoreExecutionIntent["source"],
  resolution: RestoreResolution,
  sessionId: string,
  createdAt = Date.now(),
  revision = 1
): RestoreExecutionIntent {
  return {
    sessionId,
    source,
    selectedIds: new Set(resolution.selectedIds),
    allowedIds: new Set(resolution.executableIds),
    selectedCount: resolution.selectedCount,
    executableCount: resolution.executableCount,
    excludedCount: resolution.excludedCount,
    reasonCounts: { ...resolution.reasonCounts },
    createdAt,
    revision,
    authorityFingerprint: resolution.fingerprint
  };
}

/**
 * This is the final security intersection used immediately before invoking a
 * restore API. The caller may provide a larger current selection, but only
 * IDs in the immutable confirmation allow-list and the latest authoritative
 * executable set survive.
 */
export function resolveRestoreExecutionIds(
  currentSelectedIds: ReadonlySet<string> | readonly string[],
  intent: Pick<RestoreExecutionIntent, "allowedIds">,
  authoritativeRestorableIds: ReadonlySet<string> | readonly string[]
) {
  const selected = uniqueIds(currentSelectedIds);
  const allowed = new Set(intent.allowedIds);
  const authoritative = new Set(authoritativeRestorableIds);
  return selected.filter((id) => allowed.has(id) && authoritative.has(id));
}

export function restoreIntentMatchesResolution(
  intent: RestoreExecutionIntent,
  resolution: RestoreResolution
) {
  const actualIds = resolveRestoreExecutionIds(intent.selectedIds, intent, resolution.executableIds);
  const allowedIds = new Set(intent.allowedIds);
  const allowedIdsMatch = actualIds.length === allowedIds.size && actualIds.every((id) => allowedIds.has(id));
  return allowedIdsMatch
    && resolution.selectedCount === intent.selectedCount
    && resolution.executableCount === intent.executableCount
    && resolution.excludedCount === intent.excludedCount
    && countsEqual(resolution.reasonCounts, intent.reasonCounts)
    && resolution.fingerprint === intent.authorityFingerprint;
}

export interface OperationHistoryBatch {
  id: string;
  createdAt: string;
  logs: OperationLog[];
  total: number;
  success: number;
  failed: number;
  skipped: number;
  pending: number;
  noOp: number;
  canceled: number;
  restored: number;
  restorable: number;
  excluded: number;
  executionState: HistoryExecutionState;
  restoreState: HistoryRestoreState;
  state: HistoryBatchState;
}

export type HistoryExecutionState = "success" | "partial" | "failed" | "skipped" | "canceled" | "pending" | "no_op" | "unavailable";
export type HistoryRestoreState = "not_restored" | "restorable" | "partially_restored" | "restored" | "restore_failed" | "restore_canceled" | "unavailable";

export type HistoryBatchState =
  | "success"
  | "partial"
  | "failed"
  | "skipped"
  | "canceled"
  | "pending"
  | "no_op"
  | "restorable"
  | "partially_restored"
  | "restored"
  | "restore_failed"
  | "restore_canceled"
  | "unavailable";

function timeValue(value: string | number | null | undefined) {
  if (typeof value === "number" && Number.isFinite(value) && value > 0) return value;
  const text = String(value ?? "");
  const numeric = Number(text);
  if (Number.isFinite(numeric) && numeric > 0) return numeric;
  const parsed = Date.parse(text);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function historyTime(value: string | number | null | undefined) {
  return timeValue(value);
}

/** Returns one user operation for Ctrl/Cmd+Z, retaining backend order on ties. */
export function latestUndoBatch(logs: readonly OperationLog[]) {
  const latest = [...logs].sort((a, b) => historyTime(b.created_at) - historyTime(a.created_at))[0];
  if (!latest) return [];
  return latest.batch_id
    ? logs.filter((log) => log.batch_id === latest.batch_id)
    : [latest];
}

export function resolveHistoryBatchExecutionState(logs: readonly OperationLog[]): HistoryExecutionState {
  if (!logs.length) return "unavailable";
  const pending = logs.filter((log) => log.status === "pending").length;
  const noOp = logs.filter(isNoOpLog).length;
  const success = logs.filter((log) => log.status === "success").length;
  const failed = logs.filter((log) => log.status === "failed").length;
  const skipped = logs.filter((log) => log.status === "skipped").length;
  if (pending === logs.length) return "pending";
  if (noOp === logs.length) return "no_op";
  if (pending > 0) return "partial";
  if (noOp > 0) return "partial";
  if (success === logs.length) return "success";
  if (failed === logs.length) return "failed";
  if (skipped === logs.length) return "skipped";
  if (success > 0 && failed + skipped > 0) return "partial";
  if (new Set(logs.map((log) => log.status)).size > 1) return "partial";
  return "unavailable";
}

export function resolveHistoryBatchRestoreState(logs: readonly OperationLog[]): HistoryRestoreState {
  if (!logs.length) return "unavailable";
  const restored = logs.filter((log) => log.restore_status === "restored").length;
  const failed = logs.filter((log) => log.restore_status === "failed").length;
  const canceled = logs.filter((log) => log.restore_status === "canceled").length;
  if (restored === logs.length) return "restored";
  if (failed > 0) return "restore_failed";
  if (canceled > 0) return "restore_canceled";
  if (restored > 0) return "partially_restored";
  if (logs.some(isRestorableLog)) return "restorable";
  if (logs.every((log) => log.restore_status === "not_restored" || log.restore_status === "unavailable")) {
    return logs.some((log) => log.restore_status === "unavailable") ? "unavailable" : "not_restored";
  }
  return "unavailable";
}

export function resolveHistoryBatchState(logs: readonly OperationLog[]): HistoryBatchState {
  if (!logs.length) return "unavailable";
  const executionState = resolveHistoryBatchExecutionState(logs);
  const restoreState = resolveHistoryBatchRestoreState(logs);
  if (executionState === "success") {
    if (restoreState === "restored" || restoreState === "partially_restored" || restoreState === "restorable" || restoreState === "restore_failed" || restoreState === "restore_canceled") return restoreState;
    return executionState;
  }
  // Keep an original failure/partial execution visible even when a restore
  // anomaly exists; the list renders restoreState beside this value.
  return executionState;
}

function buildOperationBatch(id: string, entries: readonly OperationLog[]): OperationHistoryBatch {
  const batchLogs = [...entries].sort((a, b) => timeValue(b.created_at) - timeValue(a.created_at));
  const executionState = resolveHistoryBatchExecutionState(batchLogs);
  const restoreState = resolveHistoryBatchRestoreState(batchLogs);
  return {
    id,
    createdAt: batchLogs[0]?.created_at ?? "",
    logs: batchLogs,
    total: batchLogs.length,
    success: batchLogs.filter((log) => log.status === "success").length,
    failed: batchLogs.filter((log) => log.status === "failed").length,
    skipped: batchLogs.filter((log) => log.status === "skipped").length,
    pending: batchLogs.filter((log) => log.status === "pending").length,
    noOp: batchLogs.filter(isNoOpLog).length,
    canceled: batchLogs.filter((log) => log.restore_status === "canceled").length,
    restored: batchLogs.filter((log) => log.restore_status === "restored").length,
    restorable: batchLogs.filter(isRestorableLog).length,
    excluded: batchLogs.filter((log) => !isRestorableLog(log)).length,
    executionState,
    restoreState,
    state: resolveHistoryBatchState(batchLogs)
  };
}

export function groupOperationLogs(logs: readonly OperationLog[]): OperationHistoryBatch[] {
  const groups = new Map<string, OperationLog[]>();
  for (const log of logs) {
    // Logs without a backend batch id are each a distinct operation. Merging
    // them into one synthetic batch loses selection and summary truthfulness.
    const id = log.batch_id || `unbatched-${log.created_at}-${log.id}`;
    const group = groups.get(id) ?? [];
    group.push(log);
    groups.set(id, group);
  }
  return [...groups.entries()]
    .map(([id, entries]) => buildOperationBatch(id, entries))
    .sort((a, b) => timeValue(b.createdAt) - timeValue(a.createdAt));
}

export type HistoryFilter =
  | "all"
  | "restorable"
  | "restored"
  | "success"
  | "failed"
  | "restoreFailed"
  | "skipped"
  | "canceled"
  | "needsReview";

export function historyFilterMatchesLog(log: OperationLog, filter: HistoryFilter) {
  if (filter === "all") return true;
  if (filter === "restorable") return isRestorableLog(log);
  if (filter === "restored") return log.restore_status === "restored";
  if (filter === "success") return log.status === "success";
  if (filter === "failed") return log.status === "failed";
  if (filter === "restoreFailed") return log.restore_status === "failed";
  if (filter === "skipped") return log.status === "skipped";
  if (filter === "canceled") return log.restore_status === "canceled";
  const reason = restoreEligibility(log).reason;
  return reason === "backendBlocked"
    || reason === "unavailable"
    || reason === "missingSource"
    || reason === "restoreFailed"
    || reason === "failedOperation";
}

export function matchesHistoryFilter(log: OperationLog, query: string) {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return true;
  return [
    log.id,
    log.batch_id,
    log.operation_type,
    log.source_path,
    log.target_path,
    log.path_before,
    log.path_after,
    log.old_name,
    log.new_name,
    log.name_before,
    log.name_after,
    log.error_message,
    log.restore_error
  ]
    .filter((value): value is string => Boolean(value))
    .some((value) => value.toLocaleLowerCase().includes(normalized));
}

export function filterHistoryBatches(
  batches: readonly OperationHistoryBatch[],
  query: string,
  filter: HistoryFilter = "all"
) {
  return batches
    .map((batch) => ({
      batch,
      logs: batch.logs.filter((log) => historyFilterMatchesLog(log, filter) && matchesHistoryFilter(log, query))
    }))
    .filter(({ logs }) => logs.length > 0)
    .map(({ batch, logs }) => buildOperationBatch(batch.id, logs));
}

export function cleanupBatchRestorableCount(
  batch: CleanupTrashBatch,
  previews: readonly CleanupRestorePreviewItem[] = [],
  authorities?: ReadonlyMap<string, CleanupPreviewAuthority>
) {
  const previewById = new Map(previews.map((item) => [item.id, item]));
  return batch.items.filter((item) => {
    const authority = authorities?.get(item.id);
    return isRestorableCleanupTrashItem(item, authority?.preview ?? previewById.get(item.id), authority?.state);
  }).length;
}

export function groupCleanupBatches(batches: readonly CleanupTrashBatch[]) {
  return [...batches].sort((a, b) => timeValue(b.createdAt) - timeValue(a.createdAt));
}

export function matchesCleanupTrashSearch(item: CleanupTrashItem, query: string) {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return true;
  return [item.id, item.batchId, item.originalPath, item.trashPath, item.name, item.message]
    .filter((value): value is string => Boolean(value))
    .some((value) => value.toLocaleLowerCase().includes(normalized));
}

export function filterCleanupBatches(
  batches: readonly CleanupTrashBatch[],
  query: string,
  previews: readonly CleanupRestorePreviewItem[] = [],
  authorities?: ReadonlyMap<string, CleanupPreviewAuthority>
) {
  const previewById = new Map(previews.map((item) => [item.id, item]));
  return batches
    .map((batch) => {
      const items = batch.items.filter((item) => matchesCleanupTrashSearch(item, query));
      return {
        ...batch,
        items,
        totalItems: items.length,
        totalSize: items.reduce((total, item) => total + item.size, 0),
        restorable: items.filter((item) => {
          const authority = authorities?.get(item.id);
          return isRestorableCleanupTrashItem(item, authority?.preview ?? previewById.get(item.id), authority?.state);
        }).length
      };
    })
    .filter((batch) => batch.items.length > 0);
}

export interface HistorySummary {
  operations: number;
  operationRestorable: number;
  cleanupRestorable: number;
  restorable: number;
  restored: number;
  unavailable: number;
}

export function resolveHistorySummary(
  logs: readonly OperationLog[],
  cleanupItems: readonly CleanupTrashItem[] = [],
  cleanupPreviews: readonly CleanupRestorePreviewItem[] = [],
  authorities?: ReadonlyMap<string, CleanupPreviewAuthority>
): HistorySummary {
  const previewById = new Map(cleanupPreviews.map((item) => [item.id, item]));
  const operationRestorable = logs.filter(isRestorableLog).length;
  const cleanupRestorable = cleanupItems.filter((item) => {
    const authority = authorities?.get(item.id);
    return isRestorableCleanupTrashItem(item, authority?.preview ?? previewById.get(item.id), authority?.state);
  }).length;
  const restored = logs.filter((log) => log.restore_status === "restored").length
    + cleanupItems.filter((item) => item.status === "restored").length;
  const unavailable = logs.filter((log) => !isRestorableLog(log) && log.restore_status !== "restored").length
    + cleanupItems.filter((item) => {
      const authority = authorities?.get(item.id);
      const preview = authority?.preview ?? previewById.get(item.id);
      const state = authority?.state ?? (preview ? "ready" : "unavailable");
      return state === "ready"
        && !isRestorableCleanupTrashItem(item, preview, state)
        && item.status !== "restored";
    }).length;
  return {
    operations: logs.length + cleanupItems.length,
    operationRestorable,
    cleanupRestorable,
    restorable: operationRestorable + cleanupRestorable,
    restored,
    unavailable
  };
}

export function reasonCountTotal(counts: Record<string, number>) {
  return Object.values(counts).reduce((total, count) => total + (count ?? 0), 0);
}
