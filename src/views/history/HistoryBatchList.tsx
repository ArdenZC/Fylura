import { useEffect, useRef, type RefObject } from "react";
import { AlertCircle, Check, ChevronRight, Circle, CircleOff, LoaderCircle, MinusCircle, RotateCcw } from "lucide-react";
import type { Translator } from "../../types/ui";
import type { OperationLog } from "../../types/domain";
import { formatCount } from "../../i18n";
import { cn, selectedFocusSurface } from "../../utils/tw";
import { compactPath, formatDisplayPath } from "../../utils/viewHelpers";
import { historyTime, isNoOpLog, isRestorableLog, type OperationHistoryBatch } from "./historyModel";

function executionStateLabel(batch: OperationHistoryBatch, t: Translator) {
  if (batch.executionState === "partial") return t("historyStatusPartial");
  if (batch.executionState === "failed") return t("historyStatusFailed");
  if (batch.executionState === "skipped") return t("historyStatusSkipped");
  if (batch.executionState === "canceled") return t("historyStatusCanceled");
  if (batch.executionState === "pending") return t("historyStatusPending");
  if (batch.executionState === "no_op") return t("historyStatusNoOp");
  if (batch.executionState === "success") return t("historyStatusSuccess");
  return t("historyStatusUnavailable");
}

function restoreStateLabel(batch: OperationHistoryBatch, t: Translator) {
  if (batch.restoreState === "restored") return t("historyStatusRestored");
  if (batch.restoreState === "partially_restored") return t("historyStatusPartiallyRestored");
  if (batch.restoreState === "restorable") return t("historyStatusRestorable");
  if (batch.restoreState === "restore_failed") return t("historyStatusRestoreFailed");
  if (batch.restoreState === "restore_canceled") return t("historyStatusRestoreCanceled");
  if (batch.restoreState === "not_restored") return t("historyStatusNotRestored");
  return t("historyStatusUnavailable");
}

function operationSourceLabel(log: OperationLog | undefined, t: Translator) {
  if (!log) return t("historyOperationSourceUnknown");
  if (isNoOpLog(log)) return t("historyStatusNoOp");
  if (log.operation_type === "move") return t("operationMove");
  if (log.operation_type === "rename") return t("operationRename");
  if (log.operation_type === "move_rename") return t("operationMoveRename");
  if (log.operation_type === "copy") return t("operationCopy");
  if (log.operation_type === "duplicate") return t("operationDuplicate");
  if (log.operation_type === "replace") return t("operationReplace");
  if (log.operation_type === "permanent_delete") return t("operationPermanentDelete");
  if (log.operation_type === "move_to_trash") return t("operationMoveToTrash");
  if (log.operation_type === "create_directory") return t("operationCreateDirectory");
  if (log.operation_type === "create_file") return t("operationCreateFile");
  return t("historyOperationSourceUnknown");
}

function BatchStateIcon({ state }: { state: OperationHistoryBatch["state"] }) {
  if (state === "restored") return <Check size={15} aria-hidden="true" />;
  if (state === "partially_restored" || state === "partial" || state === "restore_failed" || state === "restore_canceled") return <AlertCircle size={15} aria-hidden="true" />;
  if (state === "failed") return <CircleOff size={15} aria-hidden="true" />;
  if (state === "pending") return <LoaderCircle size={15} aria-hidden="true" className="animate-spin motion-reduce:animate-none" />;
  if (state === "no_op") return <MinusCircle size={15} aria-hidden="true" />;
  if (state === "restorable") return <RotateCcw size={15} aria-hidden="true" />;
  return <Circle size={15} aria-hidden="true" />;
}

function BatchCheckbox({
  batch,
  selectedIds,
  onChange,
  t
}: {
  batch: OperationHistoryBatch;
  selectedIds: ReadonlySet<string>;
  onChange: (checked: boolean) => void;
  t: Translator;
}) {
  const ref = useRef<HTMLInputElement | null>(null);
  const selectable = batch.logs.filter(isRestorableLog);
  const selected = selectable.filter((log) => selectedIds.has(log.id)).length;
  const selectedAny = selectable.some((log) => selectedIds.has(log.id));
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = selected > 0 && selected < selectable.length;
  }, [selectable.length, selected]);
  return (
    <input
      ref={ref}
      type="checkbox"
      aria-label={t("historySelectBatch")}
      checked={selectable.length > 0 && selected === selectable.length}
      disabled={selectable.length === 0 && !selectedAny}
      onChange={(event) => onChange(event.currentTarget.checked)}
      onClick={(event) => event.stopPropagation()}
    />
  );
}

export function HistoryBatchList({
  batches,
  activeBatchId,
  selectedIds,
  onActiveBatch,
  onToggleBatch,
  listRef,
  t
}: {
  batches: readonly OperationHistoryBatch[];
  activeBatchId: string;
  selectedIds: ReadonlySet<string>;
  onActiveBatch: (id: string) => void;
  onToggleBatch: (batch: OperationHistoryBatch, checked: boolean) => void;
  listRef?: RefObject<HTMLDivElement | null>;
  t: Translator;
}) {
  const activeIndex = Math.max(0, batches.findIndex((batch) => batch.id === activeBatchId));
  const internalListRef = useRef<HTMLDivElement | null>(null);
  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!batches.length) return;
    let nextIndex = activeIndex;
    if (event.key === "ArrowDown") nextIndex = Math.min(batches.length - 1, activeIndex + 1);
    else if (event.key === "ArrowUp") nextIndex = Math.max(0, activeIndex - 1);
    else if (event.key === "PageDown") nextIndex = Math.min(batches.length - 1, activeIndex + 5);
    else if (event.key === "PageUp") nextIndex = Math.max(0, activeIndex - 5);
    else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = batches.length - 1;
    else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      const batch = batches[activeIndex];
      if (batch) onToggleBatch(batch, batch.logs.filter(isRestorableLog).some((log) => !selectedIds.has(log.id)));
      return;
    } else return;
    event.preventDefault();
    const batch = batches[nextIndex];
    if (batch) {
      onActiveBatch(batch.id);
      requestAnimationFrame(() => document.getElementById(`history-batch-${batch.id}`)?.scrollIntoView?.({ block: "nearest" }));
    }
  };
  return (
    <div
      ref={listRef ?? internalListRef}
      className="grid gap-1 pr-1"
      role="listbox"
      tabIndex={0}
      aria-label={t("historyBatches")}
      aria-activedescendant={activeBatchId ? `history-batch-${activeBatchId}` : undefined}
      onKeyDown={handleKeyDown}
    >
      {batches.map((batch) => {
        const selected = batch.logs.filter((log) => isRestorableLog(log) && selectedIds.has(log.id)).length;
        const active = batch.id === activeBatchId;
        const first = batch.logs[0];
        return (
          <div
            id={`history-batch-${batch.id}`}
            key={batch.id}
            role="option"
            aria-selected={active}
            className={cn(
              "grid grid-cols-[auto_minmax(0,1fr)_auto] items-start gap-3 rounded-[var(--zc-radius-field)] border px-3 py-3 text-left transition-colors",
              active
                 ? selectedFocusSurface
                : "border-transparent hover:border-[var(--zc-border)] hover:bg-[var(--zc-surface-raised)]"
            )}
            onClick={() => onActiveBatch(batch.id)}
          >
            <BatchCheckbox batch={batch} selectedIds={selectedIds} onChange={(checked) => onToggleBatch(batch, checked)} t={t} />
            <div className="min-w-0">
              <div className="flex min-w-0 items-center gap-2">
                <span className={cn("shrink-0", batch.executionState === "failed" || batch.restoreState === "restore_failed" ? "text-[var(--zc-danger-text)]" : batch.restoreState === "restored" ? "text-[var(--zc-success-text)]" : "text-[var(--zc-primary)]")}><BatchStateIcon state={batch.state} /></span>
                <strong className="truncate text-sm">{historyTime(batch.createdAt) ? new Date(historyTime(batch.createdAt)).toLocaleString() : t("historyTimeUnavailable")}</strong>
              </div>
              <span className="mt-1 line-clamp-2 block min-w-0 break-words text-xs text-[var(--muted)] [overflow-wrap:anywhere]" title={first?.path_after || first?.target_path} aria-label={first ? formatDisplayPath(first.path_after || first.target_path) : t("historyBatch")}>
                {first ? compactPath(formatDisplayPath(first.path_after || first.target_path), 88) : t("historyBatch")}
              </span>
              <span className="mt-1 block text-xs text-[var(--muted)]">
                {formatCount(t, batch.total, { zero: "historyBatchItemsZero", one: "historyBatchItemsOne", other: "historyBatchItemsOther" })} · {operationSourceLabel(first, t)} · {executionStateLabel(batch, t)} · {restoreStateLabel(batch, t)} · {batch.restorable} {t("restorable")}
              </span>
              {(batch.failed > 0 || batch.skipped > 0 || batch.pending > 0 || batch.noOp > 0) && <span className="mt-1 block text-[11px] tabular-nums text-[var(--muted)]">{batch.failed > 0 ? `${t("historyStatusFailed")}: ${batch.failed}` : ""}{batch.skipped > 0 ? ` · ${t("historyStatusSkipped")}: ${batch.skipped}` : ""}{batch.pending > 0 ? ` · ${t("historyStatusPending")}: ${batch.pending}` : ""}{batch.noOp > 0 ? ` · ${t("historyStatusNoOp")}: ${batch.noOp}` : ""}</span>}
            </div>
            <ChevronRight size={16} className="mt-1 text-[var(--muted)]" aria-hidden="true" />
            {selected > 0 && <span className="col-start-2 text-xs font-medium text-[var(--zc-primary)]">{t("historyBatchSelected").replace("{count}", String(selected))}</span>}
          </div>
        );
      })}
    </div>
  );
}

export function operationDisplayName(log: OperationLog) {
  return log.name_after || log.new_name || log.name_before || log.old_name || log.path_after || log.target_path;
}
