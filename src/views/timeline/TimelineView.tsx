import { ManualRenameDialog } from "../fileLibrary/library/ManualRenameDialog";
import { manualMoveDestinationDirectory } from "../../api/operationApi";
import { useVirtualizer } from "@tanstack/react-virtual";
import { CheckCircle2, CircleSlash2, Folder, Play, TriangleAlert, X } from "lucide-react";
import { useRef, useState } from "react";
import type { OperationProgressPayload } from "../../api/tauriApi";
import { useI18nContext, useNavigationContext } from "../../contexts/AppContexts";
import { operationConfirmationTone, operationNeedsCleanupConfirmation, previewsForExecutionIntent, resolveExecutableSelectedPreviews, resolvePreviewEligibility, selectionForPreviewGroup, useOperationQueueStore } from "../../store/useOperationQueueStore";
import type { OperationPreview } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { groupOperationPreviews, compactPath, formatDisplayPath, libraryScopeLabel } from "../../utils/viewHelpers";
import { useFileMutationUnavailableCode } from "../../utils/fileMutationCapability";
import { buttonSecondary, cn, contentSurface, focusVisibleState, buttonDefault, buttonPrimary, buttonWarning, raisedSurface } from "../../utils/tw";
import {
  ConfirmDialog,
  NoticeBanner,
  StateBlock,
  contentPanel,
  interactiveRow,
  pageSurface,
  panelSurface,
  sectionDescription,
  sectionHeading,
  softPanel
} from "../shared/ui";
import { operationResultState, summarizeOperationLogs } from "../organize/organizeModel";
import { PreviewFileRow } from "./PreviewFileRow";

export function TimelineView() {
  const { t } = useI18nContext();
  const { setView } = useNavigationContext();
  const displayPreviews = useOperationQueueStore((state) => state.displayPreviews);
  const executionIntent = useOperationQueueStore((state) => state.executionIntent);
  const previewScope = useOperationQueueStore((state) => state.previewScope);
  const previewTotal = useOperationQueueStore((state) => state.previewTotal);
  const previewTruncated = useOperationQueueStore((state) => state.previewTruncated);
  const previewHasMore = useOperationQueueStore((state) => state.previewHasMore);
  const selectedIds = useOperationQueueStore((state) => state.selectedOperationIds);
  const setSelectedIds = useOperationQueueStore((state) => state.setSelectedOperationIds);
  const loadMorePreviews = useOperationQueueStore((state) => state.loadMorePreviews);
  const onRenamePreview = useOperationQueueStore((state) => state.onRenamePreview);
  const materializePreview = useOperationQueueStore((state) => state.materializePreview);
  const executeSelected = useOperationQueueStore((state) => state.executeSelected);
  const lastExecutionLogs = useOperationQueueStore((state) => state.lastExecutionLogs);
  const executionError = useOperationQueueStore((state) => state.executionError);
  const operationProgress = useOperationQueueStore((state) => state.operationProgress);
  const isOperationCanceling = useOperationQueueStore((state) => state.isOperationCanceling);
  const cancelOperations = useOperationQueueStore((state) => state.cancelOperations);
  const [editingManual, setEditingManual] = useState<OperationPreview | null>(null);
  const adjustManualRef = useRef<HTMLButtonElement | null>(null);
  const [confirmExecute, setConfirmExecute] = useState(false);
  const [showSafetyDetails, setShowSafetyDetails] = useState(false);
  const mutationUnavailable = useFileMutationUnavailableCode();
  const executeButtonRef = useRef<HTMLButtonElement | null>(null);
  const visiblePreviews = previewsForExecutionIntent(displayPreviews, executionIntent);
  const manualPreview = visiblePreviews.length === 1 && /^op-manual-(rename|move)-/.test(visiblePreviews[0].id) ? visiblePreviews[0] : null;
  function toggle(id: string) {
    const preview = visiblePreviews.find((item) => item.id === id);
    if (!preview || (!selectedIds.has(id) && !resolvePreviewEligibility(preview, executionIntent).executable)) return;
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setSelectedIds(next);
  }

  const groups = groupOperationPreviews(visiblePreviews, t);
  const executableCount = visiblePreviews.filter((preview) => resolvePreviewEligibility(preview, executionIntent).executable).length;
  const blockedCount = visiblePreviews.filter((preview) => resolvePreviewEligibility(preview, executionIntent).reason === "blocked").length;
  const confirmationCount = visiblePreviews.filter((preview) => preview.requires_confirmation).length;
  const autoCreateParentCount = visiblePreviews.filter(
    (preview) => preview.operation_type !== "move_to_trash" && preview.will_create_parent
  ).length;
  const executeProgress = operationProgress?.kind === "execute" ? operationProgress : null;
  const materializeProgress = operationProgress?.kind === "materialize" ? operationProgress : null;
  const isExecuting = Boolean(executeProgress || materializeProgress);
  const scopeText = previewScope
    ? libraryScopeLabel(previewScope, t("allIndexedFiles"), t("noFolderSelected"))
    : null;
  const coveredTotal = executionIntent?.source === "organize" ? visiblePreviews.length : previewTotal || visiblePreviews.length;
  const executionSelection = resolveExecutableSelectedPreviews(displayPreviews, selectedIds, executionIntent);
  const selectedCount = executionSelection.selectedCount;
  const selectedOperations = executionSelection.operations;
  const executableSelectedCount = selectedOperations.length;
  const confirmationTone = operationConfirmationTone(selectedOperations);
  const trashSelectionCount = selectedOperations.filter((preview) => preview.operation_type === "move_to_trash").length;
  const sensitiveSelectionCount = selectedOperations.filter((preview) => preview.risk_level === "Sensitive").length;
  const systemSelectionCount = selectedOperations.filter((preview) => preview.risk_level === "System").length;
  const duplicateSelectionCount = selectedOperations.filter((preview) => preview.is_duplicate).length;
  const confirmationSelectionCount = selectedOperations.filter((preview) => preview.requires_confirmation).length;
  const createParentSelectionCount = selectedOperations.filter((preview) => preview.will_create_parent).length;
  const lowConfidenceSelectionCount = selectedOperations.filter((preview) => preview.confidence < 0.7).length;
  const warningSelectionCount = selectedOperations.filter(operationNeedsCleanupConfirmation).length;
  const selectedAttentionCount = selectedOperations.filter((preview) => {
    const eligibility = resolvePreviewEligibility(preview, executionIntent);
    return !eligibility.executable || operationNeedsCleanupConfirmation(preview);
  }).length;
  const resultSummary = summarizeOperationLogs(lastExecutionLogs);
  const resultState = operationResultState(resultSummary, Boolean(executionError));
  const ResultIcon = resultState === "success" ? CheckCircle2 : resultState === "canceled" || resultState === "no-changes" ? CircleSlash2 : TriangleAlert;
  const executeButtonLabel = t("executeSelectedWithCount").replace("{count}", executableSelectedCount.toLocaleString());
  const confirmationTitle = confirmationTone === "danger"
    ? t("confirmMoveToTrashTitle")
    : confirmationTone === "warning"
      ? t("organizeExecuteRiskConfirmTitle")
      : t("organizeExecuteNormalConfirmTitle");
  const confirmationEmphasis = confirmationTone === "danger"
    ? t("organizeTrashConfirmEmphasis").replace("{count}", trashSelectionCount.toLocaleString())
    : sensitiveSelectionCount > 0
      ? t("organizeSensitiveConfirmEmphasis").replace("{count}", sensitiveSelectionCount.toLocaleString())
      : confirmationTone === "warning"
        ? t("organizeRiskConfirmEmphasis").replace("{count}", warningSelectionCount.toLocaleString())
        : t("organizeNormalConfirmEmphasis");

  return (
    <div className={pageSurface}>
      <section className={panelSurface}>
        <div className="mb-4 flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0">
            {executionIntent?.source === "organize" ? <button className={cn(buttonSecondary, "mb-3 min-h-8 px-3 py-1.5 text-xs")} onClick={() => setView("organize")}>{t("organizeBackToSuggestions")}</button> : null}
            <h2 className={sectionHeading}>{t("suggestedPlan")}</h2>
            <p className={sectionDescription}>{t("previewBeforeExecute")}</p>
            {executionIntent?.source === "organize" ? <p className="mt-1 text-sm text-[var(--zc-info-text)]">{t("organizePreviewAcceptedOnly")}</p> : null}
            {scopeText ? <p className="mt-2 truncate text-xs text-[var(--zc-text-secondary)]">{t("currentOrganizeScope")}: {scopeText}</p> : null}
          </div>
          <button ref={executeButtonRef} data-dialog-focus-fallback className={cn(buttonPrimary, "tabular-nums")} onClick={() => setConfirmExecute(true)} disabled={!executableSelectedCount || isExecuting || Boolean(mutationUnavailable)} title={mutationUnavailable ? t("errorMacosFileMutationSourceBindingUnsupported") : undefined}>
            <Play size={16} />
            <span>{isExecuting ? t("executingOperations") : executeButtonLabel}</span>
          </button>
        </div>

        <div className="mb-4 grid gap-3">
          {manualPreview && !isExecuting ? <NoticeBanner tone={manualPreview.blocking_reason ? "warning" : "info"}>
            <p>{manualPreview.blocking_reason === "manual_file_target_exists" ? t("libraryManualConflict") : t("libraryManualAdjustDescription")}</p>
            <div className="mt-2 flex flex-wrap gap-2">
              <button ref={adjustManualRef} className={buttonSecondary} onClick={() => { setSelectedIds(new Set()); setConfirmExecute(false); setEditingManual(manualPreview); }}>{t("libraryManualAdjust")}</button>
              <button className={buttonSecondary} onClick={() => { setSelectedIds(new Set()); setView("library"); }}>{t("cancel")}</button>
            </div>
          </NoticeBanner> : null}
          {mutationUnavailable && <NoticeBanner tone="warning">{t("errorMacosFileMutationSourceBindingUnsupported")}</NoticeBanner>}
          {executionIntent?.source === "organize" && visiblePreviews.length < executionIntent.initialAllowedCount ? <NoticeBanner tone="warning">{t("organizePreviewInvalidated")}</NoticeBanner> : null}
          <NoticeBanner tone="warning" title={t("previewSafetyTitle")}>
            {visiblePreviews.some((preview) => preview.operation_type === "permanent_delete") ? t("previewPermanentDeleteSafety") : t("previewNoOverwriteDelete")}
          </NoticeBanner>
          {visiblePreviews.some((preview) => preview.operation_type === "move_to_trash") && (
            <NoticeBanner tone="warning">
              {t("previewCleanupTrashSafety")}
            </NoticeBanner>
          )}
          <dl className={cn(contentSurface, "grid grid-cols-2 divide-x divide-y divide-[var(--zc-divider)] overflow-hidden text-sm sm:grid-cols-4 sm:divide-y-0")}>
            <PreviewCount label={t("previewSummarySelected")} value={selectedCount} />
            <PreviewCount label={t("previewSummaryExecutable")} value={executableSelectedCount} />
            <PreviewCount label={t("previewSummaryNeedsAttention")} value={selectedAttentionCount} />
            <PreviewCount label={t("previewSummaryImpact")} value={coveredTotal} />
          </dl>
          <details className={cn(contentSurface, "group overflow-hidden")} open={showSafetyDetails} onToggle={(event) => setShowSafetyDetails(event.currentTarget.open)}>
            <summary className={cn("flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-2 text-sm font-medium text-[var(--zc-text-primary)] [&::-webkit-details-marker]:hidden", focusVisibleState)}>
              <span>{t("previewSafetyDetails")}</span>
              <span aria-hidden="true" className="text-xs text-[var(--zc-text-tertiary)] transition-transform motion-reduce:transition-none group-open:rotate-180">⌄</span>
            </summary>
            <div className="grid gap-3 border-t border-[var(--zc-divider)] p-3">
              <dl className="grid grid-cols-2 divide-x divide-y divide-[var(--zc-divider)] overflow-hidden text-sm sm:grid-cols-3 sm:divide-y-0">
                <PreviewCount label={t("previewTotalSuggestions")} value={coveredTotal} />
                <PreviewCount label={t("executableItems")} value={executableCount} />
                <PreviewCount label={t("blockedItems")} value={blockedCount} />
                <PreviewCount label={t("confirmationItems")} value={confirmationCount} />
                <PreviewCount label={t("autoCreateFolders")} value={autoCreateParentCount} />
                <PreviewCount label={t("previewLowConfidence")} value={lowConfidenceSelectionCount} />
              </dl>
              {previewTruncated ? <NoticeBanner tone="warning">{t("previewDetailsTruncated").replace("{shown}", visiblePreviews.length.toLocaleString()).replace("{total}", coveredTotal.toLocaleString())}</NoticeBanner> : null}
              {autoCreateParentCount > 0 ? <NoticeBanner tone="info">{t("autoCreateFolderHint").replace("{count}", autoCreateParentCount.toLocaleString())}</NoticeBanner> : null}
            </div>
          </details>
        </div>
        {executionSelection.excludedCount > 0 ? (
          <NoticeBanner tone="warning" title={t("organizeSelectionExcludedTitle")}>
            {t("organizeSelectionExcludedDesc")
              .replace("{selected}", selectedCount.toLocaleString())
              .replace("{executable}", executableSelectedCount.toLocaleString())
              .replace("{invalid}", executionSelection.invalidNameCount.toLocaleString())
              .replace("{blocked}", executionSelection.blockedCount.toLocaleString())
              .replace("{outside}", executionSelection.outsideWhitelistCount.toLocaleString())
              .replace("{unavailable}", executionSelection.unavailableCount.toLocaleString())}
          </NoticeBanner>
        ) : null}
        {(executeProgress || materializeProgress) && (
          <OperationProgressPanel
            progress={executeProgress ?? materializeProgress!}
            isCanceling={isOperationCanceling}
            onCancel={cancelOperations}
            t={t}
          />
        )}
        {!isExecuting && (lastExecutionLogs.length || executionError) ? (
          <section className={cn(raisedSurface, "mb-4 grid gap-3 p-4")} aria-labelledby="organize-result-title">
            <div className="flex items-start gap-3">
              <ResultIcon className={cn("mt-0.5 shrink-0", resultState === "success" ? "text-[var(--zc-success-text)]" : resultState === "failed" ? "text-[var(--zc-danger-text)]" : "text-[var(--zc-warning-text)]")} size={20} aria-hidden="true" />
              <div>
                <h3 id="organize-result-title" className="font-semibold text-[var(--zc-text-primary)]">{t(resultState === "success" ? "organizeResultSuccessTitle" : resultState === "partial" ? "organizeResultPartialTitle" : resultState === "failed" ? "organizeResultFailedTitle" : resultState === "canceled" ? "organizeResultCanceledTitle" : "organizeResultNoChangesTitle")}</h3>
                {lastExecutionLogs.length ? <p className="mt-1 text-sm text-[var(--zc-text-secondary)]">{t("organizeResultSummary").replace("{success}", resultSummary.success.toLocaleString()).replace("{skipped}", resultSummary.skipped.toLocaleString()).replace("{failed}", resultSummary.failed.toLocaleString())}</p> : null}
                {executionError ? <p className="mt-1 text-sm text-[var(--zc-danger-text)]">{executionError}</p> : null}
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              <button className={buttonSecondary} onClick={() => setView("restore")}>{t("organizeViewHistory")}</button>
              {resultSummary.restorable ? <button className={buttonSecondary} onClick={() => setView("restore")}>{t("organizeRestoreEntry").replace("{count}", resultSummary.restorable.toLocaleString())}</button> : null}
            </div>
          </section>
        ) : null}
        {!visiblePreviews.length ? (
          <StateBlock
            title={t("previewEmptyTitle")}
            description={t("previewEmptyDesc")}
            primaryAction={(
              <button className={buttonPrimary} onClick={() => setView("organize")}>
                {t("goSmartDispatch")}
              </button>
            )}
            secondaryAction={(
              <button className={buttonDefault} onClick={() => setView("automation")}>
                {t("goAutomationWorkspace")}
              </button>
            )}
          />
        ) : (
          <div className="grid gap-4">
            {groups.map((group) => {
              const selectable = group.items.filter((preview) => resolvePreviewEligibility(preview, executionIntent).executable);
              const allSelected = selectable.length > 0 && selectable.every((item) => selectedIds.has(item.id));
              const selectedInGroupIds = new Set(group.items.filter((item) => selectedIds.has(item.id)).map((item) => item.id));
              const selectedInGroup = selectedInGroupIds.size;
              const executableSelectedInGroup = resolveExecutableSelectedPreviews(group.items, selectedInGroupIds, executionIntent).operations.length;
              const groupDisabledDescriptionId = `group-disabled-${group.key}`;
              return (
                <section className={cn(interactiveRow(), selectable.length === 0 && "opacity-75", "grid gap-3 p-4")} key={group.key}>
                  <label className={cn("grid grid-cols-[auto_auto_minmax(0,1fr)_auto] items-center gap-3", selectable.length > 0 && "cursor-pointer")}>
                    <input
                      type="checkbox"
                      checked={allSelected}
                      disabled={selectable.length === 0}
                      aria-describedby={selectable.length === 0 ? groupDisabledDescriptionId : undefined}
                      onChange={() => {
                        const shouldSelect = !allSelected;
                        setSelectedIds(selectionForPreviewGroup(selectedIds, group.items, shouldSelect, executionIntent));
                      }}
                    />
                    <Folder size={20} />
                    <div>
                      <strong className="block text-sm">{group.displayName}</strong>
                    <span className="block truncate text-xs text-[var(--zc-text-secondary)]" title={group.displayPath}>{group.displayPath}</span>
                    </div>
                    <em className="rounded-full border border-[var(--zc-border)] px-2 py-1 text-xs not-italic tabular-nums text-[var(--zc-text-secondary)]" title={t("organizeGroupSelectionSummary").replace("{selected}", selectedInGroup.toLocaleString()).replace("{executable}", executableSelectedInGroup.toLocaleString())}>
                      {t("organizeGroupSelectionCompact").replace("{selected}", selectedInGroup.toLocaleString()).replace("{executable}", executableSelectedInGroup.toLocaleString())}
                    </em>
                  </label>
                  {selectable.length === 0 && (
                    <p id={groupDisabledDescriptionId} className="text-xs text-[var(--zc-text-secondary)]">
                      {t("groupNoExecutableItems")}
                    </p>
                  )}
                  <div className="grid gap-3">
                    {group.subgroups.map((subgroup) => (
                      <section className={cn(softPanel, "p-3")} key={`${group.key}-${subgroup.key}`}>
                        <div className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 pb-2">
                          <Folder size={16} />
                          <div>
                            <strong className="block text-sm">{subgroup.displayName}</strong>
                            <span className="block truncate text-xs text-[var(--zc-text-secondary)]" title={subgroup.displayPath}>{subgroup.displayPath}</span>
                          </div>
                          <em className="text-xs not-italic text-[var(--zc-text-secondary)]">{subgroup.items.length}</em>
                        </div>
                        <VirtualPreviewFileRows
                          previews={subgroup.items}
                          selectedIds={selectedIds}
                          executionIntent={executionIntent}
                          toggle={toggle}
                          onRenamePreview={onRenamePreview}
                          onMaterialize={materializePreview}
                          busy={isExecuting}
                          t={t}
                        />
                      </section>
                    ))}
                  </div>
                </section>
              );
            })}
            {previewHasMore && (
              <button className={buttonDefault} onClick={loadMorePreviews}>
                {t("loadMoreFiles").replace("{count}", Math.max(0, coveredTotal - visiblePreviews.length).toLocaleString())}
              </button>
            )}
          </div>
        )}
      </section>
      {editingManual ? <ManualRenameDialog
        key={editingManual.id}
        file={{ id: editingManual.fileId, name: editingManual.old_name }}
        mode={editingManual.id.startsWith("op-manual-move-") ? "move" : "rename"}
        initialDirectory={editingManual.id.startsWith("op-manual-move-") ? manualMoveDestinationDirectory(editingManual) : ""}
        initialName={editingManual.new_name}
        t={t} restoreFocus={() => adjustManualRef.current}
        onClose={() => setEditingManual(null)}
        onPreview={(preview) => {
          useOperationQueueStore.getState().setPreviewResult({ previews: [preview], total: 1, limit: 1, offset: 0, truncated: false, hasMore: false }, previewScope ?? { kind: "all" });
          setEditingManual(null);
        }}
      /> : null}
      <ConfirmDialog
        open={confirmExecute}
        tone={confirmationTone}
        title={confirmationTitle}
        description={[
          confirmationTone === "danger" ? t("confirmMoveToTrashDesc") : confirmationTone === "warning" ? t("organizeExecuteRiskConfirmDesc") : t("organizeExecuteNormalConfirmDesc"),
          t("organizeExecuteConfirmDesc").replace("{count}", selectedOperations.length.toLocaleString()),
          confirmationTone !== "default" ? t("organizeExecuteRiskSummary")
            .replace("{confirmation}", confirmationSelectionCount.toLocaleString())
            .replace("{sensitive}", sensitiveSelectionCount.toLocaleString())
            .replace("{system}", systemSelectionCount.toLocaleString())
            .replace("{duplicate}", duplicateSelectionCount.toLocaleString())
            .replace("{trash}", trashSelectionCount.toLocaleString())
            .replace("{folders}", createParentSelectionCount.toLocaleString())
            .replace("{lowConfidence}", lowConfidenceSelectionCount.toLocaleString()) : ""
        ].filter(Boolean).join("\n")}
        emphasis={confirmationEmphasis}
        confirmLabel={t("organizeExecuteConfirmAction").replace("{count}", selectedOperations.length.toLocaleString())}
        cancelLabel={t("cancel")}
        restoreFocus={() => executeButtonRef.current}
        disabled={Boolean(mutationUnavailable)}
        onCancel={() => setConfirmExecute(false)}
        onConfirm={() => { setConfirmExecute(false); void executeSelected(true); }}
      />
    </div>
  );
}

function PreviewCount({ label, value }: { label: string; value: number }) {
  return <div className="flex items-baseline justify-between gap-2 px-3 py-2"><dt className="text-[var(--zc-text-secondary)]">{label}</dt><dd className="font-semibold tabular-nums text-[var(--zc-text-primary)]">{value.toLocaleString()}</dd></div>;
}

function VirtualPreviewFileRows({
  previews,
  selectedIds,
  executionIntent,
  toggle,
  onRenamePreview,
  onMaterialize,
  busy,
  t
}: {
  previews: OperationPreview[];
  selectedIds: Set<string>;
  executionIntent: import("../../store/useOperationQueueStore").PreviewExecutionIntent;
  toggle: (id: string) => void;
  onRenamePreview: (id: string, name: string) => void;
  onMaterialize: (preview: OperationPreview) => Promise<void>;
  busy: boolean;
  t: Translator;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const virtualizer = useVirtualizer({
    count: previews.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 92,
    overscan: 6
  });
  if (previews.length <= 20) {
    return (
      <div className="grid gap-3">
        {previews.map((preview) => (
          <PreviewFileRow key={preview.id} preview={preview} isSelected={selectedIds.has(preview.id)} executionIntent={executionIntent} toggle={toggle} onRenamePreview={onRenamePreview} onMaterialize={onMaterialize} busy={busy} t={t} />
        ))}
      </div>
    );
  }
  return (
    <div ref={scrollRef} className="max-h-[520px] overflow-auto" role="list">
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const preview = previews[virtualRow.index];
          return (
            <div
              key={preview.id}
              ref={virtualizer.measureElement}
              data-index={virtualRow.index}
              className="absolute left-0 top-0 w-full pb-3"
              style={{ transform: `translateY(${virtualRow.start}px)` }}
              role="listitem"
            >
              <PreviewFileRow preview={preview} isSelected={selectedIds.has(preview.id)} executionIntent={executionIntent} toggle={toggle} onRenamePreview={onRenamePreview} onMaterialize={onMaterialize} busy={busy} t={t} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function OperationProgressPanel({
  progress,
  isCanceling,
  onCancel,
  t
}: {
  progress: OperationProgressPayload;
  isCanceling: boolean;
  onCancel: () => Promise<void>;
  t: Translator;
}) {
  const ratio = progress.total > 0 ? Math.min(1, progress.processed / progress.total) : 0;
  const currentPath = progress.currentPath ? compactPath(formatDisplayPath(progress.currentPath), 56) : "-";
  const line = t("operationProgressLine")
    .replace("{processed}", progress.processed.toLocaleString())
    .replace("{total}", progress.total.toLocaleString())
    .replace("{path}", currentPath);
  const progressLabel = progress.kind === "restore"
    ? t("restoring")
    : progress.kind === "materialize"
      ? t("operationMaterializationDownload")
      : t("operationProgressTitle");

  return (
    <div className={cn(contentPanel, "mb-4 grid gap-3 p-4")} role="status" aria-live="polite">
      <div className="flex items-center justify-between gap-3 text-sm">
        <strong>{progressLabel}</strong>
        <span className="text-[var(--zc-text-secondary)]">
          {progress.processed.toLocaleString()} / {progress.total.toLocaleString()}
        </span>
      </div>
      <div
        className="h-2 overflow-hidden rounded-full bg-white/50 dark:bg-white/10"
        role="progressbar"
        aria-label={progressLabel}
        aria-valuemin={0}
        aria-valuemax={progress.total}
        aria-valuenow={Math.min(progress.total, progress.processed)}
      >
        <div
          className="h-full rounded-full bg-[var(--zc-primary)] transition-[width] motion-reduce:transition-none"
          style={{ width: `${Math.round(ratio * 100)}%` }}
        />
      </div>
      <div className="flex min-w-0 items-center justify-between gap-3">
        <small className="min-w-0 truncate text-xs text-[var(--zc-text-secondary)]" title={progress.currentPath ?? undefined}>{line}</small>
        <button className={buttonWarning} onClick={onCancel} disabled={isCanceling}>
          <X size={15} />
          <span>{isCanceling ? t("operationCanceling") : t("cancel")}</span>
        </button>
      </div>
    </div>
  );
}
