import { HardDrive, ListChecks, FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";
import { tauriApi, type ManagedScanEvent, type ScanRootDto } from "../../api/tauriApi";
import { requestSettingsSection } from "../../components/spotlight/commandRegistry";
import { useI18nContext, useNavigationContext } from "../../contexts/AppContexts";
import { useBackgroundIndexerStore } from "../../store/useBackgroundIndexerStore";
import { useFileLibraryStore } from "../../store/useFileLibraryStore";
import { organizationPlanReviewCount, selectReviewableOrganizationPlan, useOrganizationPlanStore } from "../../store/useOrganizationPlanStore";
import { useOperationQueueStore } from "../../store/useOperationQueueStore";
import { useScanManagerStore } from "../../store/useScanManagerStore";
import { formatBytes } from "../../utils/format";
import { resolveReclaimableBytes } from "../../utils/reclaimableBytes";
import { cn } from "../../utils/tw";
import { summarizeWatcherHealth, watcherHealthAttentionCount } from "../../utils/watcherPresentation";
import type { AnalysisRun } from "../../types/domain";
import { pageSurface } from "../shared/ui";
import { OverviewPriorityTask } from "../overview/OverviewPriorityTask";
import { ScanTaskPanel } from "../overview/ScanTaskPanel";
import { ScanCancelDialog } from "../overview/ScanCancelDialog";
import {
  OverviewBackgroundTaskList,
  OverviewQuickActions,
  OverviewRecentActivityList,
  OverviewSpaceSummary,
  OverviewSystemCoverage,
  type OverviewQuickAction,
  type OverviewSystemCoverageModel
} from "../overview/OverviewSections";
import {
  buildOverviewSummary,
  deriveOverviewScanState,
  selectOverviewBackgroundTasks,
  selectOverviewPriorityTask,
  selectRecentOverviewActivity,
  type OverviewHealthSnapshot,
  type OverviewPriorityTaskModel
} from "../overview/overviewModel";

const terminalScanStatuses = new Set([
  "cancelled",
  "cancelled_not_started",
  "completed",
  "completed_with_warnings",
  "failed",
  "interrupted",
  "requires_reconciliation"
]);
const terminalAnalysisStatuses = new Set([
  "cancelled",
  "completed",
  "completed_with_warnings",
  "failed",
  "interrupted"
]);
const OVERVIEW_HEALTH_SAFETY_REFRESH_MS = 60_000;

export function ScannerView() {
  const { t, language } = useI18nContext();
  const { setView } = useNavigationContext();
  const [isCancelDialogOpen, setIsCancelDialogOpen] = useState(false);
  const scope = useFileLibraryStore((state) => state.scope);
  const stats = useFileLibraryStore((state) => state.stats);
  const isClassifyingWithAI = useFileLibraryStore((state) => state.isClassifyingWithAI);
  const aiClassificationProgress = useFileLibraryStore((state) => state.aiClassificationProgress);
  const selectedFolders = useScanManagerStore((state) => state.selectedFolders);
  const isScanning = useScanManagerStore((state) => state.isScanning);
  const isCancelingScan = useScanManagerStore((state) => state.isCancelingScan);
  const scanState = useScanManagerStore((state) => state.scanState);
  const handleChooseFolders = useScanManagerStore((state) => state.handleChooseFolders);
  const handleScan = useScanManagerStore((state) => state.handleScan);
  const cancelScan = useScanManagerStore((state) => state.cancelScan);
  const operationLogs = useOperationQueueStore((state) => state.operationLogs);
  const operationProgress = useOperationQueueStore((state) => state.operationProgress);
  const pendingRoots = useBackgroundIndexerStore((state) => state.pendingRoots);
  const currentBackgroundRoot = useBackgroundIndexerStore((state) => state.currentRoot);
  const isBackgroundIndexing = useBackgroundIndexerStore((state) => state.isBackgroundIndexing);
  const failedRoots = useBackgroundIndexerStore((state) => state.failedRoots);
  const enqueueBackgroundRoot = useBackgroundIndexerStore((state) => state.enqueueRoot);
  const activePlan = useOrganizationPlanStore((state) => state.activePlan);
  const plans = useOrganizationPlanStore((state) => state.plans);
  const loadPlans = useOrganizationPlanStore((state) => state.loadPlans);
  const [globalIndexStatus, setGlobalIndexStatus] = useState<Awaited<ReturnType<typeof tauriApi.getGlobalIndexStatus>> | null>(null);
  const [scanRoots, setScanRoots] = useState<ScanRootDto[]>([]);
  const [managedScopes, setManagedScopes] = useState<Awaited<ReturnType<typeof tauriApi.listManagedScopes>>>([]);
  const [activeAnalysisRun, setActiveAnalysisRun] = useState<Awaited<ReturnType<typeof tauriApi.getActiveAnalysisRun>>>(null);
  const [analysisRuns, setAnalysisRuns] = useState<Awaited<ReturnType<typeof tauriApi.listAnalysisRuns>>>([]);
  const [contentRuns, setContentRuns] = useState<Awaited<ReturnType<typeof tauriApi.listContentRuns>>>([]);
  const [contentCoverage, setContentCoverage] = useState({ known: false, enabled: 0, total: 0 });

  useEffect(() => {
    void loadPlans();
  }, [loadPlans]);

  useEffect(() => {
    let disposed = false;
    let healthRefreshEpoch = 0;
    let healthSafetyRefreshTimer: number | null = null;
    const unlisteners: Array<() => void> = [];
    const lastWatcherState = new Map<string, string>();
    const refreshHealth = async () => {
      const refreshEpoch = ++healthRefreshEpoch;
      const [indexResult, rootsResult, managedScopesResult, analysisResult, analysisRunsResult, contentResult] = await Promise.allSettled([
        tauriApi.getGlobalIndexStatus(),
        tauriApi.listScanRoots(),
        tauriApi.listManagedScopes(),
        tauriApi.getActiveAnalysisRun(),
        tauriApi.listAnalysisRuns(20),
        tauriApi.listContentRuns(10)
      ]);
      const isCurrentRefresh = () => !disposed && refreshEpoch === healthRefreshEpoch;
      if (!isCurrentRefresh()) return;
      if (indexResult.status === "fulfilled") setGlobalIndexStatus(indexResult.value);
      if (rootsResult.status === "fulfilled") setScanRoots(rootsResult.value);
      if (managedScopesResult.status === "fulfilled") setManagedScopes(managedScopesResult.value);
      if (analysisResult.status === "fulfilled") setActiveAnalysisRun(analysisResult.value);
      if (analysisRunsResult.status === "fulfilled") setAnalysisRuns(analysisRunsResult.value);
      if (contentResult.status === "fulfilled") setContentRuns(contentResult.value);
      if (rootsResult.status !== "fulfilled") {
        setContentCoverage({ known: false, enabled: 0, total: 0 });
        return;
      }

      const roots = rootsResult.value;
      const enabledRoots = roots.filter((root) => root.enabled);
      const policyResults = await Promise.allSettled(
        enabledRoots.map((root) => tauriApi.getContentScopePolicy(root.id))
      );
      if (!isCurrentRefresh()) return;
      const known = policyResults.every((result) => result.status === "fulfilled");
      setContentCoverage({
        known,
        enabled: known
          ? policyResults.filter((result) => result.status === "fulfilled" && result.value.enabled).length
          : 0,
        total: roots.length
      });
    };
    const refreshIfVisible = () => {
      if (!disposed && document.visibilityState === "visible") void refreshHealth();
    };
    const clearHealthSafetyRefresh = () => {
      if (healthSafetyRefreshTimer !== null) {
        window.clearInterval(healthSafetyRefreshTimer);
        healthSafetyRefreshTimer = null;
      }
    };
    const startHealthSafetyRefresh = () => {
      clearHealthSafetyRefresh();
      if (document.visibilityState === "visible") {
        healthSafetyRefreshTimer = window.setInterval(
          refreshIfVisible,
          OVERVIEW_HEALTH_SAFETY_REFRESH_MS
        );
      }
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        refreshIfVisible();
        startHealthSafetyRefresh();
      } else {
        clearHealthSafetyRefresh();
      }
    };
    const registerUnlistener = (register: () => Promise<() => void>) => {
      try {
        void register().then((unlisten) => {
          if (disposed) unlisten();
          else unlisteners.push(unlisten);
        }).catch(() => undefined);
      } catch {
        // The initial durable snapshot remains available if an optional event source fails.
      }
    };

    if (document.visibilityState === "visible") {
      refreshIfVisible();
      startHealthSafetyRefresh();
    }
    document.addEventListener("visibilitychange", handleVisibilityChange);
    registerUnlistener(() => tauriApi.onWatcherReconciliationStatus((event) => {
      const nextState = [
        event.pending,
        event.needsReconciliation,
        event.rootRevision,
        event.watcherRevision,
        event.watcherAppliedRevision,
        event.healthStatus,
        event.activeRunId ?? "",
        event.lastErrorCode ?? "",
        event.lastErrorMessage ?? ""
      ].join(":");
      const previousState = lastWatcherState.get(event.scanRootId);
      lastWatcherState.set(event.scanRootId, nextState);
      if (previousState !== nextState) refreshIfVisible();
    }));
    registerUnlistener(() => tauriApi.onManagedScanEvent((event: ManagedScanEvent) => {
      if (terminalScanStatuses.has(event.status)) refreshIfVisible();
    }));
    registerUnlistener(() => tauriApi.onAnalysisRunUpdated((event: AnalysisRun) => {
      if (terminalAnalysisStatuses.has(event.status)) refreshIfVisible();
    }));
    registerUnlistener(() => tauriApi.onAnalysisFindingsPublished(() => refreshIfVisible()));

    return () => {
      disposed = true;
      healthRefreshEpoch += 1;
      clearHealthSafetyRefresh();
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      for (const unlisten of unlisteners) unlisten();
    };
  }, []);

  const scanSnapshot = {
    status: scanState.status,
    isScanning,
    isCanceling: isCancelingScan,
    progress: scanState.progress,
    error: scanState.error
  };
  const hasIndexedData = stats.totalFiles > 0 || stats.totalSize > 0;
  const scanVisualState = deriveOverviewScanState(scanSnapshot, hasIndexedData);
  const activities = selectRecentOverviewActivity(operationLogs, t);
  const backgroundTasks = selectOverviewBackgroundTasks({
    backgroundIndexing: isBackgroundIndexing,
    currentRoot: currentBackgroundRoot,
    pendingRoots,
    failedRoots,
    operationProgress,
    aiProgress: aiClassificationProgress ? {
      processed: aiClassificationProgress.processed,
      total: aiClassificationProgress.total,
      currentPath: aiClassificationProgress.currentFilePreview
    } : null,
    isClassifyingWithAI
  });
  const scopeRoots = scope.kind === "all" ? [] : scope.roots;
  const overviewRoots = scopeRoots.length > 0 ? scopeRoots : selectedFolders;
  const summary = buildOverviewSummary(stats, overviewRoots, t, language);
  const scanFallbackPath = scanState.progress?.root || selectedFolders[0] || scopeRoots[0] || "";
  const latestContentRun = contentRuns
    .slice()
    .sort((left, right) => right.updatedAt - left.updatedAt)[0]
    ?? null;
  const cleanupRun = (activeAnalysisRun?.scope?.kind === "approved_cleanup_paths" ? activeAnalysisRun : null)
    ?? analysisRuns
      .filter((run) => run.scope?.kind === "approved_cleanup_paths")
      .sort((left, right) => right.updatedAt - left.updatedAt)[0]
    ?? null;
  const watcherHealth = summarizeWatcherHealth(scanRoots.filter((root) => root.enabled));
  const operationAttentionCount = operationLogs.filter((log) => log.status === "failed" || log.status === "manual_review" || String(log.restore_status).includes("failed") || String(log.restore_status).includes("manual_review") || String(log.restore_status).includes("conflict")).length;
  const globalIndexNoSource = globalIndexStatus
    ? globalIndexStatus.status === "no_source"
      || (!globalIndexStatus.enabled && globalIndexStatus.indexedVolumes === 0)
      || (globalIndexStatus.status === "unavailable" && globalIndexStatus.totalEntries === 0 && globalIndexStatus.indexedVolumes === 0)
    : false;
  const overviewPlan = selectReviewableOrganizationPlan(plans, activePlan);
  const planReviewCount = overviewPlan ? organizationPlanReviewCount(overviewPlan) : stats.needsConfirmation;
  const cleanupCandidateCount = cleanupRun
    ? cleanupRun.safeCount + cleanupRun.reviewCount + cleanupRun.cautionCount
    : 0;
  const cleanupReclaimable = resolveReclaimableBytes({
    exact: cleanupRun?.exactReclaimableBytes,
    potential: cleanupRun?.potentialReclaimableBytes
  });
  const cleanupMetric = cleanupRun && cleanupReclaimable.bytes > 0
    ? {
      label: t("overviewMetricReclaimable"),
      value: `${cleanupReclaimable.estimated ? "~" : ""}${formatBytes(cleanupReclaimable.bytes)}`,
      hint: cleanupReclaimable.estimated ? t("storageCleanupEstimateHint") : undefined
    }
    : {
      label: t("overviewMetricReclaimable"),
      value: t("overviewMetricNoData")
    };
  const health: OverviewHealthSnapshot = {
    globalIndex: globalIndexStatus ? {
      status: globalIndexStatus.status,
      collectionComplete: globalIndexStatus.collectionComplete,
      lastError: globalIndexStatus.lastError,
      enabled: globalIndexStatus.enabled,
      noSource: globalIndexNoSource
    } : null,
    watcher: watcherHealth,
    plan: overviewPlan,
    cleanupRun,
    contentRun: latestContentRun,
    operation: { active: operationProgress != null, attentionCount: operationAttentionCount }
  };
  const indexNeedsUpdate = Boolean(globalIndexStatus && (
    globalIndexStatus.status !== "ready" || !globalIndexStatus.collectionComplete
  ));
  const priorityTask = selectOverviewPriorityTask({
    scan: scanSnapshot,
    stats,
    cleanupCandidateCount,
    reclaimableBytes: cleanupReclaimable.bytes,
    indexNeedsUpdate,
    health
  });
  const systemCoverage: OverviewSystemCoverageModel = {
    search: globalIndexStatus
      ? (globalIndexStatus.status === "ready" && globalIndexStatus.collectionComplete && !globalIndexNoSource ? "ready" : globalIndexNoSource || ["permission_required", "error", "unavailable", "no_source"].includes(globalIndexStatus.status) ? "attention" : "partial")
      : "unknown",
    managedCount: managedScopes.filter((scope) => scope.enabled).length,
    managedTotal: managedScopes.length,
    managedAttention: watcherHealthAttentionCount(watcherHealth),
    contentKnown: contentCoverage.known,
    contentEnabled: contentCoverage.enabled,
    contentTotal: contentCoverage.total
  };
  const overviewMetrics = [
    { label: t("overviewMetricFiles"), value: stats.totalFiles.toLocaleString() },
    { label: t("overviewMetricSuggestions"), value: planReviewCount.toLocaleString() },
    cleanupMetric
  ];
  const quickActions: OverviewQuickAction[] = [
    {
      id: "files",
      label: t("overviewQuickFiles"),
      description: t("overviewQuickFilesDesc"),
      icon: FolderOpen,
      onClick: () => setView("library")
    },
    {
      id: "suggestions",
      label: t("overviewQuickSuggestions"),
      description: planReviewCount > 0 ? t("overviewQuickSuggestionsDesc") : t("overviewQuickSuggestionsEmptyDesc"),
      icon: ListChecks,
      onClick: () => setView("organize")
    },
    {
      id: "cleanup",
      label: t("overviewQuickCleanup"),
      description: cleanupCandidateCount > 0 ? t("overviewQuickCleanupDesc") : t("overviewQuickCleanupEmptyDesc"),
      icon: HardDrive,
      onClick: () => setView("cleanup")
    }
  ];

  function runPrimaryAction(task: OverviewPriorityTaskModel) {
    if (task.kind === "search-permission") {
      setView("settings");
      requestSettingsSection("settings-global-index");
      return;
    }
    if (task.kind === "operation") {
      setView("restore");
      return;
    }
    if (task.kind === "content-failure") {
      setView("library");
      return;
    }
    if (task.kind === "managed-root-stale") {
      setView("settings");
      requestSettingsSection("settings-files-scan");
      return;
    }
    if (task.kind === "review") {
      setView("organize");
      return;
    }
    if (task.kind === "cleanup") {
      setView("cleanup");
      return;
    }
    if (task.kind === "scan-active" || task.kind === "scan-canceling" || task.kind === "scan-partial") {
      document.getElementById("overview-scan-task")?.scrollIntoView({ block: "nearest" });
      return;
    }
    if (task.kind === "orderly") {
      void handleChooseFolders();
      return;
    }
    void handleScan();
  }

  async function confirmCancelScan() {
    await cancelScan();
    setIsCancelDialogOpen(false);
  }

  return (
    <div className={cn(pageSurface, "v340-page")}>
      <OverviewPriorityTask
        task={priorityTask}
        t={t}
        onPrimary={() => runPrimaryAction(priorityTask)}
        onChooseFolder={() => void handleChooseFolders()}
        onCancel={() => setIsCancelDialogOpen(true)}
        metrics={overviewMetrics}
      />

      <OverviewQuickActions actions={quickActions} t={t} />

      <ScanTaskPanel
        state={scanVisualState}
        progress={scanState.progress}
        error={scanState.error}
        fallbackPath={scanFallbackPath}
        t={t}
        language={language}
      />

      <OverviewSpaceSummary summary={summary} t={t} />
      <OverviewSystemCoverage coverage={systemCoverage} t={t} />
      <OverviewRecentActivityList activities={activities} t={t} language={language} onOpenHistory={() => setView("restore")} />
      <OverviewBackgroundTaskList
        tasks={backgroundTasks}
        t={t}
        onRetryIndex={(path) => enqueueBackgroundRoot(path, { force: true })}
      />

      <ScanCancelDialog
        open={isCancelDialogOpen}
        isCanceling={isCancelingScan}
        t={t}
        onConfirm={confirmCancelScan}
        onCancel={() => setIsCancelDialogOpen(false)}
      />
    </div>
  );
}
