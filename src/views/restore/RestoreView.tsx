import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { RotateCcw, ShieldCheck, SlidersHorizontal, Trash2 } from "lucide-react";
import { tauriApi } from "../../api/tauriApi";
import { useI18nContext } from "../../contexts/AppContexts";
import { useMediaQuery } from "../../hooks/useMediaQuery";
import { useOperationQueueStore } from "../../store/useOperationQueueStore";
import type { CleanupRestorePreviewItem, CleanupTrashBatch, CleanupTrashItem, OperationLog, RecoveryAction } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { formatBytes } from "../../utils/format";
import { useFileMutationUnavailableCode } from "../../utils/fileMutationCapability";
import { buttonGhost, cn, contentPanel, emptyState, focusVisibleState, buttonPrimary, selectedFocusVisibleState, selectedSurface } from "../../utils/tw";
import { OperationProgressPanel } from "../timeline/TimelineView";
import { ConfirmDialog, MetricStrip, mutedText, pageSurface, panelSurface } from "../shared/ui";
import { HistoryBatchList } from "../history/HistoryBatchList";
import { CleanupInspector, HistoryInspector } from "../history/HistoryInspector";
import { HistorySearchField } from "../history/HistorySearchField";
import {
  cleanupBatchRestorableCount,
  filterCleanupBatches,
  filterHistoryBatches,
  groupCleanupBatches,
  groupOperationLogs,
  historyTime,
  resolveCleanupRestoreSelection,
  resolveHistorySummary,
  resolveOperationRestoreSelection,
  reconcileRestorableOperationSelection,
  selectionForOperationBatch,
  type HistoryFilter,
  type CleanupPreviewAuthority,
  type CleanupPreviewState,
  type OperationHistoryBatch
} from "../history/historyModel";

type ViewFilter = HistoryFilter | "cleanup";

type CleanupPreviewRecord = {
  state: CleanupPreviewState;
  items: CleanupRestorePreviewItem[];
  error: string;
};

function replace(text: string, values: Record<string, string | number>) {
  return Object.entries(values).reduce((result, [key, value]) => result.replace(`{${key}}`, String(value)), text);
}

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

export function RestoreView() {
  const { t } = useI18nContext();
  const logs = useOperationQueueStore((state) => state.operationLogs);
  const requestedCleanupBatchId = useOperationQueueStore((state) => state.requestedCleanupBatchId);
  const requestedHistoryBatchId = useOperationQueueStore((state) => state.requestedHistoryBatchId);
  const restoreIntent = useOperationQueueStore((state) => state.restoreIntent);
  const prepareOperationRestoreIntent = useOperationQueueStore((state) => state.prepareOperationRestoreIntent);
  const prepareCleanupRestoreIntent = useOperationQueueStore((state) => state.prepareCleanupRestoreIntent);
  const confirmOperationRestore = useOperationQueueStore((state) => state.confirmOperationRestore);
  const confirmCleanupRestore = useOperationQueueStore((state) => state.confirmCleanupRestore);
  const invalidateRestoreIntent = useOperationQueueStore((state) => state.invalidateRestoreIntent);
  const refreshOperationLogs = useOperationQueueStore((state) => state.refreshOperationLogs);
  const operationProgress = useOperationQueueStore((state) => state.operationProgress);
  const activeOperationKind = useOperationQueueStore((state) => state.activeOperationKind);
  const isOperationCanceling = useOperationQueueStore((state) => state.isOperationCanceling);
  const cancelOperations = useOperationQueueStore((state) => state.cancelOperations);
  const cancelCleanupRestore = useOperationQueueStore((state) => state.cancelCleanupRestore);
  const cleanupRestoreProgress = useOperationQueueStore((state) => state.cleanupRestoreProgress);
  const cleanupRestoreResult = useOperationQueueStore((state) => state.cleanupRestoreResult);
  const lastRestoreResult = useOperationQueueStore((state) => state.lastRestoreResult);
  const lastRestoreSummary = useOperationQueueStore((state) => state.lastRestoreSummary);
  const restoreError = useOperationQueueStore((state) => state.restoreError);
  const cleanupRestoreError = useOperationQueueStore((state) => state.cleanupRestoreError);
  const restoreTechnicalError = useOperationQueueStore((state) => state.restoreTechnicalError);
  const [cleanupBatches, setCleanupBatches] = useState<CleanupTrashBatch[]>([]);
  const [cleanupPreviewByBatch, setCleanupPreviewByBatch] = useState<Record<string, CleanupPreviewRecord>>({});
  const [operationLoadState, setOperationLoadState] = useState<"loading" | "ready" | "failed">("loading");
  const [cleanupLoadState, setCleanupLoadState] = useState<"loading" | "ready" | "failed">("loading");
  const [cleanupLoadError, setCleanupLoadError] = useState("");
  const [activeBatchId, setActiveBatchId] = useState("");
  const [activeCleanupBatchId, setActiveCleanupBatchId] = useState("");
  const [selectedOperationIds, setSelectedOperationIds] = useState<Set<string>>(new Set());
  const [selectedCleanupIds, setSelectedCleanupIds] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<ViewFilter>("all");
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [moreFiltersOpen, setMoreFiltersOpen] = useState(false);
  const [isPreparing, setIsPreparing] = useState(false);
  const [narrowPane, setNarrowPane] = useState<"list" | "details">("list");
  const [showTechnicalDetails, setShowTechnicalDetails] = useState(false);
  const mutationUnavailable = useFileMutationUnavailableCode();
  const confirmTriggerRef = useRef<HTMLButtonElement | null>(null);
  const pageScrollRef = useRef<HTMLDivElement | null>(null);
  const historyListRef = useRef<HTMLDivElement | null>(null);
  const cleanupListRef = useRef<HTMLDivElement | null>(null);
  const moreFiltersTriggerRef = useRef<HTMLButtonElement | null>(null);
  const moreFiltersRef = useRef<HTMLDivElement | null>(null);
  const cleanupRefreshGeneration = useRef(0);
  const isNarrow = useMediaQuery("(max-width: 1023px)");

  const operationBatches = useMemo(() => {
    const grouped = groupOperationLogs(logs);
    return filterHistoryBatches(grouped, query, filter === "cleanup" ? "all" : filter);
  }, [filter, logs, query]);
  const cleanupPreviewItems = useMemo(
    () => Object.values(cleanupPreviewByBatch).flatMap((record) => record.state === "ready" ? record.items : []),
    [cleanupPreviewByBatch]
  );
  const cleanupPreviewById = useMemo(
    () => new Map(cleanupPreviewItems.map((item) => [item.id, item])),
    [cleanupPreviewItems]
  );
  const cleanupAuthoritiesById = useMemo(() => {
    const authorities = new Map<string, CleanupPreviewAuthority>();
    for (const batch of cleanupBatches) {
      const record = cleanupPreviewByBatch[batch.id];
      const previewById = new Map((record?.items ?? []).map((item) => [item.id, item]));
      for (const item of batch.items) {
        authorities.set(item.id, {
          state: record?.state ?? "unavailable",
          preview: record?.state === "ready" ? previewById.get(item.id) : undefined,
          error: record?.error || null
        });
      }
    }
    return authorities;
  }, [cleanupBatches, cleanupPreviewByBatch]);
  const cleanup = useMemo(
    () => filterCleanupBatches(groupCleanupBatches(cleanupBatches), query, cleanupPreviewItems, cleanupAuthoritiesById),
    [cleanupAuthoritiesById, cleanupBatches, cleanupPreviewItems, query]
  );
  const activeBatch = operationBatches.find((batch) => batch.id === activeBatchId) ?? operationBatches[0];
  const activeCleanupBatch = cleanup.find((batch) => batch.id === activeCleanupBatchId) ?? cleanup[0];
  const operationSelection = useMemo(
    () => resolveOperationRestoreSelection(logs, selectedOperationIds),
    [logs, selectedOperationIds]
  );
  const cleanupSelection = useMemo(
    () => resolveCleanupRestoreSelection(cleanupBatches.flatMap((batch) => batch.items), selectedCleanupIds, cleanupAuthoritiesById),
    [cleanupAuthoritiesById, cleanupBatches, selectedCleanupIds]
  );
  const selectedCleanupItems = useMemo(
    () => cleanup.flatMap((batch) => batch.items).filter((item) => selectedCleanupIds.has(item.id)),
    [cleanup, selectedCleanupIds]
  );
  const selectedMode = selectedOperationIds.size > 0 ? "operation_logs" : selectedCleanupIds.size > 0 ? "cleanup_trash" : null;
  const selectedResolution = selectedMode === "operation_logs" ? operationSelection : cleanupSelection;
  const selectedCount = selectedResolution.selectedCount;
  const executableCount = selectedResolution.executableCount;
  const excludedCount = selectedResolution.excludedCount;
  const summary = useMemo(
    () => resolveHistorySummary(logs, cleanupBatches.flatMap((batch) => batch.items), cleanupPreviewItems, cleanupAuthoritiesById),
    [cleanupAuthoritiesById, cleanupBatches, cleanupPreviewItems, logs]
  );
  const restoreProgress = operationProgress?.kind === "restore" ? operationProgress : null;
  const busy = Boolean(restoreProgress) || Boolean(cleanupRestoreProgress) || isPreparing || activeOperationKind === "restore";
  const showCleanup = filter === "cleanup";
  const visibleError = selectedMode === "cleanup_trash" ? cleanupRestoreError : restoreError;

  const loadCleanupPreview = useCallback(async (batchId: string, generation = cleanupRefreshGeneration.current) => {
    setCleanupPreviewByBatch((current) => ({
      ...current,
      [batchId]: { state: "loading", items: [], error: "" }
    }));
    try {
      const preview = await tauriApi.previewRestoreCleanupTrash(batchId);
      if (generation !== cleanupRefreshGeneration.current) return;
      setCleanupPreviewByBatch((current) => ({
        ...current,
        [batchId]: { state: "ready", items: preview.items, error: "" }
      }));
      return true;
    } catch (error) {
      if (generation !== cleanupRefreshGeneration.current) return false;
      const message = error instanceof Error ? error.message : String(error);
      setCleanupPreviewByBatch((current) => ({
        ...current,
        [batchId]: { state: "failed", items: [], error: message }
      }));
      return false;
    }
  }, []);

  const resolveRecoveryAction = useCallback(async (log: OperationLog, action: RecoveryAction, targetPath?: string) => {
    await tauriApi.resolveOperationRecovery(log.id, action, targetPath);
    await refreshOperationLogs();
  }, [refreshOperationLogs]);

  const loadOperationHistory = useCallback(async () => {
    setOperationLoadState("loading");
    try {
      await refreshOperationLogs();
      setOperationLoadState("ready");
    } catch {
      setOperationLoadState("failed");
    }
  }, [refreshOperationLogs]);

  const refreshCleanup = useCallback(async () => {
    const generation = cleanupRefreshGeneration.current + 1;
    cleanupRefreshGeneration.current = generation;
    try {
      setCleanupLoadState("loading");
      setCleanupLoadError("");
      const batches = await tauriApi.listCleanupTrashBatches();
      if (generation !== cleanupRefreshGeneration.current) return;
      setCleanupLoadState("ready");
      setCleanupBatches(batches);
      setCleanupPreviewByBatch(Object.fromEntries(batches.map((batch) => [batch.id, { state: "loading", items: [], error: "" }])));
      const results = await Promise.all(batches.map((batch) => loadCleanupPreview(batch.id, generation)));
      if (generation === cleanupRefreshGeneration.current && results.some((result) => result === false)) {
        setCleanupLoadError(t("cleanupPreviewFailed"));
      }
    } catch (error) {
      if (generation !== cleanupRefreshGeneration.current) return;
      setCleanupLoadState("failed");
      setCleanupLoadError(error instanceof Error ? error.message : String(error));
      setCleanupBatches([]);
      setCleanupPreviewByBatch({});
    }
  }, [loadCleanupPreview, t]);

  useEffect(() => {
    void loadOperationHistory();
    void refreshCleanup();
  }, [loadOperationHistory, refreshCleanup]);

  useEffect(() => {
    if (!activeBatchId || !operationBatches.some((batch) => batch.id === activeBatchId)) setActiveBatchId(operationBatches[0]?.id ?? "");
  }, [activeBatchId, operationBatches]);

  useEffect(() => {
    if (requestedHistoryBatchId && operationBatches.some((batch) => batch.id === requestedHistoryBatchId)) {
      setActiveBatchId(requestedHistoryBatchId);
      useOperationQueueStore.setState({ requestedHistoryBatchId: null });
    }
  }, [requestedHistoryBatchId, operationBatches]);

  useEffect(() => {
    if (!requestedCleanupBatchId || !cleanupBatches.some((batch) => batch.id === requestedCleanupBatchId)) return;
    setQuery("");
    setFilter("cleanup");
    setActiveCleanupBatchId(requestedCleanupBatchId);
    if (isNarrow) setNarrowPane("details");
    useOperationQueueStore.setState({ requestedCleanupBatchId: null });
  }, [requestedCleanupBatchId, cleanupBatches, isNarrow]);

  useEffect(() => {
    if (requestedCleanupBatchId) return;
    if (!activeCleanupBatchId || !cleanup.some((batch) => batch.id === activeCleanupBatchId)) setActiveCleanupBatchId(cleanup[0]?.id ?? "");
  }, [activeCleanupBatchId, cleanup, requestedCleanupBatchId]);

  useEffect(() => {
    if (!isNarrow) setNarrowPane("list");
  }, [isNarrow]);

  useEffect(() => {
    pageScrollRef.current?.scrollTo?.({ top: 0 });
  }, [activeBatch?.id, activeCleanupBatch?.id, filter]);

  useEffect(() => {
    setSelectedOperationIds(new Set());
    setSelectedCleanupIds(new Set());
    invalidateRestoreIntent();
  }, [filter, invalidateRestoreIntent, query]);

  useEffect(() => {
    invalidateRestoreIntent();
    setSelectedOperationIds((current) => {
      const next = reconcileRestorableOperationSelection(logs, current);
      if (next.size === current.size && [...next].every((id) => current.has(id))) return current;
      return next;
    });
  }, [invalidateRestoreIntent, logs]);

  useEffect(() => () => invalidateRestoreIntent(), [invalidateRestoreIntent]);

  useEffect(() => {
    if (!isNarrow || narrowPane !== "details" || confirmOpen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setNarrowPane("list");
      requestAnimationFrame(() => (showCleanup ? cleanupListRef.current : historyListRef.current)?.focus());
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [confirmOpen, isNarrow, narrowPane, showCleanup]);

  useEffect(() => {
    if (!moreFiltersOpen) return;
    const firstFilter = moreFiltersRef.current?.querySelector<HTMLElement>('[role="dialog"] button');
    requestAnimationFrame(() => firstFilter?.focus());
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setMoreFiltersOpen(false);
      requestAnimationFrame(() => moreFiltersTriggerRef.current?.focus());
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [moreFiltersOpen]);

  function returnToList() {
    setNarrowPane("list");
    requestAnimationFrame(() => (showCleanup ? cleanupListRef.current : historyListRef.current)?.focus());
  }

  function toggleBatch(batch: OperationHistoryBatch, checked: boolean) {
    invalidateRestoreIntent();
    setSelectedCleanupIds(new Set());
    setSelectedOperationIds((current) => selectionForOperationBatch(current, batch.logs, checked));
  }

  function toggleOperation(log: OperationLog, checked: boolean) {
    invalidateRestoreIntent();
    setSelectedCleanupIds(new Set());
    setSelectedOperationIds((current) => {
      const next = new Set(current);
      if (checked) next.add(log.id);
      else next.delete(log.id);
      return next;
    });
  }

  function toggleCleanup(item: CleanupTrashItem, checked: boolean) {
    if ((cleanupPreviewByBatch[item.batchId]?.state ?? "unavailable") !== "ready") return;
    invalidateRestoreIntent();
    setSelectedOperationIds(new Set());
    setSelectedCleanupIds((current) => {
      const next = new Set(current);
      if (checked) next.add(item.id);
      else next.delete(item.id);
      return next;
    });
  }

  function changeFilter(next: ViewFilter) {
    setFilter(next);
    setMoreFiltersOpen(false);
    setSelectedOperationIds(new Set());
    setSelectedCleanupIds(new Set());
    invalidateRestoreIntent();
  }

  function changeMoreFilter(next: ViewFilter) {
    changeFilter(next);
    requestAnimationFrame(() => moreFiltersTriggerRef.current?.focus());
  }

  async function prepareConfirmation() {
    if (busy) return;
    setIsPreparing(true);
    setShowTechnicalDetails(false);
    try {
      const intent = selectedMode === "operation_logs"
        ? await prepareOperationRestoreIntent(selectedOperationIds)
        : selectedMode === "cleanup_trash"
          ? await prepareCleanupRestoreIntent(selectedCleanupItems)
          : null;
      if (intent) setConfirmOpen(true);
    } finally {
      setIsPreparing(false);
    }
  }

  async function confirmRestore() {
    if (!restoreIntent) return;
    const outcome = restoreIntent.source === "operation_logs"
      ? await confirmOperationRestore(restoreIntent.sessionId)
      : await confirmCleanupRestore(restoreIntent.sessionId);
    if (outcome.status !== "executed") return;
    setConfirmOpen(false);
    if (restoreIntent.source === "operation_logs") setSelectedOperationIds(new Set());
    else {
      setSelectedCleanupIds(new Set());
      await refreshCleanup();
    }
  }

  function cancelConfirmation() {
    setConfirmOpen(false);
    invalidateRestoreIntent();
  }

  const primaryFilterButtons: Array<{ value: ViewFilter; key: Parameters<typeof t>[0] }> = [
    { value: "all", key: "historyFilterAll" },
    { value: "restorable", key: "historyFilterRestorable" },
    { value: "needsReview", key: "historyFilterNeedsAttention" }
  ];
  const moreFilterButtons: Array<{ value: ViewFilter; key: Parameters<typeof t>[0] }> = [
    { value: "restored", key: "historyFilterRestored" },
    { value: "success", key: "historyFilterSuccess" },
    { value: "failed", key: "historyFilterFailed" },
    { value: "restoreFailed", key: "historyFilterRestoreFailed" },
    { value: "skipped", key: "historyFilterSkipped" },
    { value: "canceled", key: "historyFilterCanceled" },
    { value: "needsReview", key: "historyFilterNeedsReview" },
    { value: "cleanup", key: "historyFilterCleanup" }
  ];
  const intentCount = restoreIntent?.executableCount ?? executableCount;
  const intentExcluded = restoreIntent?.excludedCount ?? excludedCount;
  const confirmDescription = restoreIntent
    ? replace(t("historyRestoreConfirmDesc"), { count: intentCount })
      + (intentExcluded ? `\n${replace(t("historyRestoreConfirmExcluded"), { count: intentExcluded })}` : "")
    : "";
  const confirmError = visibleError && restoreIntent ? `\n${visibleError}` : undefined;

  return (
    <div ref={pageScrollRef} className={cn(pageSurface, "overflow-x-hidden overflow-y-auto") }>
      <div className="mx-auto grid min-h-full max-w-[1500px] content-start gap-5 pb-4">
        <header className="flex justify-end">
          <div className="flex max-w-xl items-start gap-2 rounded-[var(--zc-radius-control)] border border-[var(--zc-neutral-border)] bg-[var(--zc-neutral-soft)] px-3 py-2 text-xs leading-5 text-[var(--zc-neutral-text)]"><ShieldCheck size={16} className="mt-0.5 shrink-0" aria-hidden="true" />{t("historySafetyBoundary")}</div>
        </header>

        <MetricStrip
          ariaLabel={t("historyWorkspaceTitle")}
          density="compact"
          items={[
            { label: t("historySummaryOperations"), value: summary.operations },
            { label: t("historySummaryRestorable"), value: summary.restorable },
            { label: t("historySummaryRestored"), value: summary.restored },
            { label: t("historySummaryExcluded"), value: summary.unavailable }
          ]}
        />

        <section className={cn(panelSurface, "grid gap-4 overflow-visible p-4 lg:grid-cols-[minmax(260px,0.8fr)_minmax(0,1.2fr)]")}>
          <div className={cn("grid min-w-0 content-start gap-3", isNarrow && narrowPane === "details" && "hidden")}>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h2 className="text-sm font-semibold">{showCleanup ? t("historyCleanupScope") : t("historyBatches")}</h2>
              <span className="text-xs text-[var(--muted)] tabular-nums">{showCleanup ? cleanup.length : operationBatches.length}</span>
            </div>
            <HistorySearchField mode={showCleanup ? "cleanup" : "operation"} value={query} onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("historySearchPlaceholder")} />
            <div className="flex max-w-full flex-wrap items-center gap-1" role="group" aria-label={t("historyBatches")}>
              {primaryFilterButtons.map(({ value, key }) => <button key={value} type="button" aria-pressed={filter === value} className={cn(buttonGhost, "shrink-0", filter === value && selectedFocusVisibleState, filter === value && selectedSurface)} onClick={() => changeFilter(value)}>{t(key)}</button>)}
              <div ref={moreFiltersRef} className="relative">
                <button ref={moreFiltersTriggerRef} type="button" className={cn(buttonGhost, "shrink-0", !primaryFilterButtons.some((item) => item.value === filter) && selectedFocusVisibleState, !primaryFilterButtons.some((item) => item.value === filter) && selectedSurface)} aria-haspopup="dialog" aria-expanded={moreFiltersOpen} aria-controls="history-more-filters" onClick={() => setMoreFiltersOpen((current) => !current)}>
                  <SlidersHorizontal size={14} aria-hidden="true" />{t("historyMoreFilters")}
                </button>
                {moreFiltersOpen ? <div id="history-more-filters" role="dialog" aria-label={t("historyMoreFilters")} className="absolute left-0 top-full z-20 mt-2 grid min-w-52 gap-1 rounded-[var(--zc-radius-panel)] border border-[var(--zc-border)] bg-[var(--zc-surface-overlay)] p-2 shadow-[var(--zc-shadow-menu)]">
                  {moreFilterButtons.map(({ value, key }) => <button key={value} type="button" aria-pressed={filter === value} className={cn(buttonGhost, "justify-start", filter === value && selectedFocusVisibleState, filter === value && selectedSurface)} onClick={() => changeMoreFilter(value)}>{t(key)}</button>)}
                </div> : null}
              </div>
            </div>
            {!showCleanup && operationLoadState === "loading" && <div className={cn(emptyState, "gap-2")} role="status" aria-live="polite"><span>{t("historyLoading")}</span></div>}
            {!showCleanup && operationLoadState === "failed" && <div className={cn(emptyState, "grid gap-3")} role="alert"><span>{t("historyLoadFailed")}</span><button type="button" className={buttonGhost} onClick={() => void loadOperationHistory()}>{t("historyRetryLoad")}</button></div>}
            {!showCleanup && operationLoadState === "ready" && operationBatches.length > 0 && <HistoryBatchList listRef={historyListRef} batches={operationBatches} activeBatchId={activeBatch?.id ?? ""} selectedIds={selectedOperationIds} onActiveBatch={(id) => { setActiveBatchId(id); if (isNarrow) setNarrowPane("details"); }} onToggleBatch={toggleBatch} t={t} />}
            {!showCleanup && operationLoadState === "ready" && operationBatches.length === 0 && <div className={emptyState}>{query ? t("historyNoMatches") : t("historyNoRecords")}</div>}
            {showCleanup && cleanupLoadState === "loading" && <div className={cn(emptyState, "gap-2")} role="status" aria-live="polite"><span>{t("historyLoading")}</span></div>}
            {showCleanup && cleanupLoadState === "failed" && <div className={cn(emptyState, "grid gap-3")} role="alert"><span>{t("historyLoadFailed")}</span><button type="button" className={buttonGhost} onClick={() => void refreshCleanup()}>{t("historyRetryLoad")}</button></div>}
            {showCleanup && cleanupLoadState === "ready" && cleanupLoadError && <div className="flex flex-wrap items-center justify-between gap-2 rounded-[var(--zc-radius-control)] border border-[var(--zc-warning-border)] bg-[var(--zc-warning-soft)] px-3 py-2 text-xs text-[var(--zc-warning-text)]" role="status"><span>{t("cleanupPreviewFailed")}</span><button type="button" className={buttonGhost} onClick={() => void refreshCleanup()}>{t("cleanupPreviewRetry")}</button></div>}
            {showCleanup && cleanupLoadState === "ready" && cleanup.length > 0 && <div ref={cleanupListRef} className="grid gap-2" tabIndex={0}>{cleanup.map((batch) => {
              const previewState = cleanupPreviewByBatch[batch.id]?.state ?? "unavailable";
              const previewFailed = previewState === "failed" || previewState === "unavailable";
              const active = batch.id === activeCleanupBatch?.id;
              return <button key={batch.id} data-cleanup-history-batch-id={batch.id} type="button" className={cn(rowButton, active && selectedFocusVisibleState, active && selectedSurface)} onClick={() => { setActiveCleanupBatchId(batch.id); if (isNarrow) setNarrowPane("details"); }}><span className="min-w-0 text-left"><strong className="block text-sm">{formatDate(batch.createdAt, t)}</strong><span className="block truncate text-xs text-[var(--muted)]">{batch.totalItems} · {formatBytes(batch.totalSize)} · {previewFailed ? t("cleanupPreviewUnavailable") : previewState === "loading" ? t("cleanupPreviewLoading") : `${cleanupBatchRestorableCount(batch, cleanupPreviewItems, cleanupAuthoritiesById)} ${t("restorable")}`}</span></span><Trash2 size={15} aria-hidden="true" /></button>;
            })}</div>}
            {showCleanup && cleanupLoadState === "ready" && cleanup.length === 0 && <div className={emptyState}>{query ? t("historyNoMatches") : t("cleanupTrashEmpty")}</div>}
          </div>

          <div className={cn("min-w-0", isNarrow && narrowPane === "list" && "hidden")}>
            {showCleanup
              ? <CleanupInspector batch={activeCleanupBatch} previewById={cleanupPreviewById} previewState={activeCleanupBatch ? cleanupPreviewByBatch[activeCleanupBatch.id]?.state ?? "unavailable" : "unavailable"} previewError={activeCleanupBatch ? cleanupPreviewByBatch[activeCleanupBatch.id]?.error : undefined} onRetry={activeCleanupBatch ? () => void loadCleanupPreview(activeCleanupBatch.id) : undefined} onBack={isNarrow ? returnToList : undefined} selectedIds={selectedCleanupIds} onToggle={toggleCleanup} t={t} />
              : <HistoryInspector batch={activeBatch} selectedIds={selectedOperationIds} onToggle={toggleOperation} onRecoveryAction={resolveRecoveryAction} onBack={isNarrow ? returnToList : undefined} t={t} />}
          </div>
        </section>

        {selectedCount > 0 && <div className={cn(contentPanel, "sticky bottom-3 z-10 flex flex-wrap items-center justify-between gap-3 p-3 shadow-[var(--zc-shadow-float)]")}><div className="min-w-0"><strong className="block text-sm tabular-nums">{replace(t("historySelectedCount"), { count: selectedCount })}</strong><span className="block text-xs text-[var(--muted)] tabular-nums">{mutationUnavailable ? t("errorMacosFileMutationSourceBindingUnsupported") : `${replace(t("historyExecutableCount"), { count: executableCount })} · ${replace(t("historyExcludedCount"), { count: excludedCount })}`}</span></div>{executableCount > 0 ? <button ref={confirmTriggerRef} type="button" className={buttonPrimary} disabled={busy || Boolean(mutationUnavailable)} title={mutationUnavailable ? t("errorMacosFileMutationSourceBindingUnsupported") : undefined} onClick={() => void prepareConfirmation()}><RotateCcw size={16} />{busy ? t("restoring") : replace(t("historyRestoreActionCount"), { count: executableCount })}</button> : null}</div>}

        {restoreProgress && <OperationProgressPanel progress={restoreProgress} isCanceling={isOperationCanceling} onCancel={cancelOperations} t={t} />}
        {cleanupRestoreProgress && <section className={cn(contentPanel, "grid gap-2 p-4")} aria-live="polite"><strong className="text-sm">{t("historyCleanupProgressTitle")}</strong><p className={mutedText}>{replace(t("historyCleanupProgressLine"), { processed: cleanupRestoreProgress.processed, total: cleanupRestoreProgress.total, path: cleanupRestoreProgress.currentPath || cleanupRestoreProgress.currentItemId || "-" })}</p><p className={cn(mutedText, "tabular-nums")}>{replace(t("historyCleanupProgressCounts"), { restored: cleanupRestoreProgress.restored, conflicts: cleanupRestoreProgress.conflicts, missing: cleanupRestoreProgress.missing, failed: cleanupRestoreProgress.failed, canceled: cleanupRestoreProgress.canceled })}</p><button type="button" className="justify-self-start text-sm text-[var(--zc-primary)] disabled:opacity-60" disabled={cleanupRestoreProgress.cancelRequested} onClick={() => void cancelCleanupRestore()}>{cleanupRestoreProgress.cancelRequested ? t("operationCanceling") : t("cancel")}</button></section>}

        {(lastRestoreResult.length > 0 || cleanupRestoreResult || lastRestoreSummary || visibleError || restoreTechnicalError) && <section className={cn(contentPanel, "grid gap-2 p-4")} aria-live="polite"><strong className="text-sm">{t("historyRestoreResultTitle")}</strong>{cleanupRestoreResult && lastRestoreSummary ? <p className={mutedText}>{replace(t("historyCleanupRestoreResultLine"), { restored: lastRestoreSummary.restored, conflicts: lastRestoreSummary.conflicts, missing: lastRestoreSummary.missing, failed: lastRestoreSummary.failed, canceled: lastRestoreSummary.canceled, excluded: lastRestoreSummary.excluded })}</p> : lastRestoreSummary ? <p className={mutedText}>{replace(t("historyRestoreResultLine"), { restored: lastRestoreSummary.restored, failed: lastRestoreSummary.failed, skipped: lastRestoreSummary.skipped, canceled: lastRestoreSummary.canceled, excluded: lastRestoreSummary.excluded })}</p> : null}{visibleError && <p className="text-sm text-[var(--zc-danger-text)]">{visibleError}</p>}{restoreTechnicalError && <div><button type="button" className="text-xs text-[var(--zc-primary)]" aria-expanded={showTechnicalDetails} onClick={() => setShowTechnicalDetails((value) => !value)}>{showTechnicalDetails ? t("historyRestoreHideTechnical") : t("historyRestoreShowTechnical")}</button>{showTechnicalDetails && <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-words rounded-[var(--zc-radius-control)] bg-[var(--zc-surface-subtle)] p-2 text-[11px] text-[var(--muted)]">{restoreTechnicalError}</pre>}</div>}</section>}
      </div>

      <ConfirmDialog disabled={Boolean(mutationUnavailable)} open={confirmOpen} tone={restoreIntent?.source === "cleanup_trash" ? "warning" : "default"} title={t("historyRestoreConfirmTitle")} description={confirmDescription + (confirmError ?? "")} emphasis={restoreIntent ? `${replace(t("historySelectedCount"), { count: restoreIntent.selectedCount })} · ${replace(t("historyExecutableCount"), { count: restoreIntent.executableCount })} · ${replace(t("historyExcludedCount"), { count: restoreIntent.excludedCount })}` : undefined} confirmLabel={replace(t("historyRestoreActionCount"), { count: intentCount })} cancelLabel={t("cancel")} isProcessing={busy} restoreFocus={() => confirmTriggerRef.current} onCancel={cancelConfirmation} onConfirm={() => void confirmRestore()} />
    </div>
  );
}

const rowButton = cn(
  "flex w-full items-center justify-between gap-3 rounded-[var(--zc-radius-field)] border border-transparent px-3 py-2 text-left transition-colors hover:border-[var(--zc-border)] hover:bg-[var(--zc-surface-raised)]",
  focusVisibleState
);
