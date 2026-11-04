import { requestOrganizationHistoryBatch, useOrganizationOperationProgress } from "./organizationOperationFeedback";
import { OneOffOrganizeConsent } from "./OneOffOrganizeConsent";
import { useOneOffOrganizeTask } from "./useOneOffOrganizeTask";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, ChevronRight, CircleMinus, Edit3, History, ListRestart, LoaderCircle, MoreHorizontal, Play, Plus, RefreshCw, Sparkles, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { tauriApi } from "../../api/tauriApi";
import { useCommandContext, useI18nContext, useNavigationContext } from "../../contexts/AppContexts";
import { useFileLibraryQueryStore, useFileLibraryResultStore, useFileLibrarySelectionStore } from "../../store/useFileLibraryV2Store";
import { isHistoricalOrganizationPlan, useOrganizationPlanStore } from "../../store/useOrganizationPlanStore";
import type { AIProductFeatureReadiness, OrganizationPlanGroupSummary, OrganizationPlanItem, OrganizationPlanStatus, LibrarySelectionV1 } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { useFileMutationUnavailableCode } from "../../utils/fileMutationCapability";
import { formatBytes } from "../../utils/format";
import { readableError } from "../../utils/viewHelpers";
import { validateOrganizeFileNameForOriginal } from "./organizeModel";
import { buttonGhost, cn, focusVisibleState, inputSurface, selectedFocusSurface, selectedFocusVisibleState, selectedSurface } from "../../utils/tw";
import {
  Button,
  ConfirmDialog,
  DurableTaskStatus,
  MetricStrip,
  NoticeBanner,
  SegmentedControl,
  SideSheet,
  StateBlock,
  WorkflowSteps,
  type WorkflowStepState,
  pageFrame
} from "../shared/ui";
import { AIReadinessNotice, organizationPlanReadiness } from "../shared/AIReadinessNotice";

const GROUP_ROW_HEIGHT = 194;
const GROUP_PAGE_SIZE = 100;

export function organizationExecutionBatchSummary(executableCount: number, executionBatchLimit: number) {
  const totalCount = Math.max(0, Math.floor(executableCount));
  const batchLimit = Math.max(0, Math.floor(executionBatchLimit));
  const batchCount = Math.min(totalCount, batchLimit);
  return {
    batchCount,
    remainingCount: totalCount - batchCount,
    isBatched: totalCount > batchCount
  };
}

type ReviewTab = "plan" | "decision" | "blocked";

type DryRunErrorOwner = {
  planId: string;
  planRevision: number;
};

type ItemAcceptanceConfirmation = {
  planId: string;
  planRevision: number;
  itemId: string;
  item: OrganizationPlanItem;
};

type GroupAcceptanceConfirmation = {
  planId: string;
  planRevision: number;
  groupId: string;
  group: OrganizationPlanGroupSummary;
};

type ExecutionConfirmation = {
  planId: string;
  planRevision: number;
  dryRunFingerprint: string;
  batch: ReturnType<typeof organizationExecutionBatchSummary>;
};

type ConfirmedExecutionBatch = ExecutionConfirmation;

export function OrganizeSuggestionsView() {
  const { t } = useI18nContext();
  const { platform } = useCommandContext();
  const oneOff = useOneOffOrganizeTask(t, platform);
  const { setView } = useNavigationContext();
  const plans = useOrganizationPlanStore((state) => state.plans);
  const plan = useOrganizationPlanStore((state) => state.activePlan);
  const groups = useOrganizationPlanStore((state) => state.groups);
  const groupHasMore = useOrganizationPlanStore((state) => state.groupHasMore);
  const groupNextCursor = useOrganizationPlanStore((state) => state.groupNextCursor);
  const dryRun = useOrganizationPlanStore((state) => state.dryRun);
  const executionResult = useOrganizationPlanStore((state) => state.executionResult);
  const planListState = useOrganizationPlanStore((state) => state.planListState);
  const activePlanState = useOrganizationPlanStore((state) => state.activePlanState);
  const openPlanErrorPlanId = useOrganizationPlanStore((state) => state.openPlanErrorPlanId);
  const createPlanError = useOrganizationPlanStore((state) => state.createPlanError);
  const isPlanListLoading = useOrganizationPlanStore((state) => state.isPlanListLoading);
  const isLoading = useOrganizationPlanStore((state) => state.isLoading);
  const storeIsMutating = useOrganizationPlanStore((state) => state.isMutating);
  const isMutating = storeIsMutating || oneOff.reading || Boolean(oneOff.progress);
  const isExecutionInFlight = useOrganizationPlanStore((state) => state.isExecutionInFlight);
  const operationProgress = useOrganizationOperationProgress();
  const error = useOrganizationPlanStore((state) => state.error);
  const loadPlans = useOrganizationPlanStore((state) => state.loadPlans);
  const createPlan = useOrganizationPlanStore((state) => state.createPlan);
  const openPlan = useOrganizationPlanStore((state) => state.openPlan);
  const loadNextGroupPage = useOrganizationPlanStore((state) => state.loadNextGroupPage);
  const updateGroupDecision = useOrganizationPlanStore((state) => state.updateGroupDecision);
  const updateDecision = useOrganizationPlanStore((state) => state.updateDecision);
  const refreshPlan = useOrganizationPlanStore((state) => state.refreshPlan);
  const analyzeMissing = useOrganizationPlanStore((state) => state.analyzeMissing);
  const createDryRun = useOrganizationPlanStore((state) => state.createDryRun);
  const executeDryRun = useOrganizationPlanStore((state) => state.executeDryRun);
  const cancelPlan = useOrganizationPlanStore((state) => state.cancelPlan);
  const librarySelection = useFileLibrarySelectionStore((state) => state.selection);
  const querySpec = useFileLibraryQueryStore((state) => state.spec);
  const queryFingerprint = useFileLibraryQueryStore((state) => state.fingerprint);
  const querySnapshotRevision = useFileLibraryQueryStore((state) => state.snapshotRevision);
  const totalCount = useFileLibraryResultStore((state) => state.totalCount);
  const [activeTab, setActiveTab] = useState<ReviewTab>("plan");
  const [activeGroupId, setActiveGroupId] = useState<string | null>(null);
  const [groupItems, setGroupItems] = useState<OrganizationPlanItem[]>([]);
  const [groupItemsCursor, setGroupItemsCursor] = useState<string | null>(null);
  const [groupItemsHasMore, setGroupItemsHasMore] = useState(false);
  const [groupItemsLoading, setGroupItemsLoading] = useState(false);
  const [groupItemsError, setGroupItemsError] = useState<string | null>(null);
  const [activeItemId, setActiveItemId] = useState<string | null>(null);
  const [editingItemId, setEditingItemId] = useState<string | null>(null);
  const [editedName, setEditedName] = useState("");
  const [editedGroupFolder, setEditedGroupFolder] = useState("");
  const [editError, setEditError] = useState<string | null>(null);
  const [planTitle, setPlanTitle] = useState("");
  const [executionConfirmation, setExecutionConfirmation] = useState<ExecutionConfirmation | null>(null);
  const [confirmedExecutionBatch, setConfirmedExecutionBatch] = useState<ConfirmedExecutionBatch | null>(null);
  const [confirmItemAcceptance, setConfirmItemAcceptance] = useState<ItemAcceptanceConfirmation | null>(null);
  const [confirmGroupAcceptance, setConfirmGroupAcceptance] = useState<GroupAcceptanceConfirmation | null>(null);
  const [reviewActionError, setReviewActionError] = useState<string | null>(null);
  const [reviewActionNeedsRefresh, setReviewActionNeedsRefresh] = useState(false);
  const [dryRunErrorOwner, setDryRunErrorOwner] = useState<DryRunErrorOwner | null>(null);
  const [aiReadiness, setAiReadiness] = useState<AIProductFeatureReadiness | null>(null);
  const [aiReadinessLoading, setAiReadinessLoading] = useState(true);
  const [aiReadinessError, setAiReadinessError] = useState(false);
  const mutationUnavailable = useFileMutationUnavailableCode();
  const groupListRef = useRef<HTMLDivElement | null>(null);
  const groupRequestEpoch = useRef(0);
  const activeGroupIdRef = useRef<string | null>(null);
  const activeItemIdRef = useRef<string | null>(null);
  const aiReadinessRequestEpoch = useRef(0);

  const refreshAiReadiness = useCallback(async () => {
    const epoch = ++aiReadinessRequestEpoch.current;
    setAiReadinessLoading(true);
    setAiReadinessError(false);
    try {
      const readiness = await tauriApi.getAIFeatureReadiness();
      if (epoch !== aiReadinessRequestEpoch.current) return;
      setAiReadiness(readiness);
    } catch {
      if (epoch !== aiReadinessRequestEpoch.current) return;
      setAiReadiness(null);
      setAiReadinessError(true);
    } finally {
      if (epoch === aiReadinessRequestEpoch.current) setAiReadinessLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshAiReadiness().catch(() => undefined);
    return () => { aiReadinessRequestEpoch.current += 1; };
  }, [refreshAiReadiness]);

  useEffect(() => {
    activeGroupIdRef.current = activeGroupId;
  }, [activeGroupId]);

  useEffect(() => {
    activeItemIdRef.current = activeItemId;
  }, [activeItemId]);

  useEffect(() => {
    setConfirmItemAcceptance((current) => current && (!plan || current.planId !== plan.id || current.planRevision !== plan.revision) ? null : current);
    setConfirmGroupAcceptance((current) => current && (!plan || current.planId !== plan.id || current.planRevision !== plan.revision) ? null : current);
    setExecutionConfirmation((current) => current && (!plan || current.planId !== plan.id || current.planRevision !== plan.revision || current.dryRunFingerprint !== dryRun?.dryRunFingerprint) ? null : current);
    setConfirmedExecutionBatch((current) => current && (!plan || current.planId !== plan.id || (dryRun && current.dryRunFingerprint !== dryRun.dryRunFingerprint)) ? null : current);
    setDryRunErrorOwner((current) => current && (!plan || current.planId !== plan.id || current.planRevision !== plan.revision) ? null : current);
  }, [dryRun?.dryRunFingerprint, plan?.id, plan?.revision]);

  useEffect(() => {
    setReviewActionError(null);
    setReviewActionNeedsRefresh(false);
  }, [plan?.id, plan?.revision]);

  useEffect(() => {
    void loadPlans().catch(() => undefined);
  }, [loadPlans]);

  useEffect(() => {
    const planToOpen = plans.find((item) => !isHistoricalOrganizationPlan(item.status));
    if (!plan && planListState === "loaded" && activePlanState === "idle" && planToOpen) void openPlan(planToOpen.id).catch(() => undefined);
  }, [activePlanState, openPlan, plan, planListState, plans]);

  const activeGroup = groups.find((group) => group.groupId === activeGroupId) ?? null;

  const visibleGroups = useMemo(() => groups.filter((group) => {
    if (activeTab === "plan") return group.readiness === "ready" || group.readiness === "reviewed";
    if (activeTab === "decision") return group.readiness === "requires-decision";
    return group.readiness === "blocked";
  }), [activeTab, groups]);

  const activeItem = groupItems.find((item) => item.id === activeItemId) ?? null;
  const canReview = Boolean(plan && ["ready", "partially_completed"].includes(plan.status));
  const canCancel = Boolean(plan && ["draft", "building", "ready", "stale"].includes(plan.status));
  const canDryRun = Boolean(plan && ["ready", "partially_completed"].includes(plan.status) && plan.summary.remainingExecutable > 0);
  const needsAnalysisCount = plan?.summary.needsAnalysis ?? 0;
  const canGenerateSemanticProposals = Boolean(
    aiReadiness?.provider.state === "ready"
    && (plan?.sourceKind === "one_off" || aiReadiness.managedScopes.some((scope) => scope.state === "ready"))
  );
  const dryRunBatch = dryRun ? organizationExecutionBatchSummary(dryRun.executableCount, dryRun.executionBatchLimit) : null;
  const effectiveSummary = plan?.effectiveSummary ?? (plan ? {
    ready: plan.summary.ready,
    reviewed: plan.summary.reviewed,
    pendingReview: plan.summary.pendingReview,
    blocked: plan.summary.blocked
  } : null);
  const organizeWorkflowCurrent = executionResult || executionConfirmation || isExecutionInFlight
    ? 3
    : dryRun
      ? 2
      : 1;
  const organizeReviewWorkflowState: WorkflowStepState = organizeWorkflowCurrent === 1 && Boolean(effectiveSummary?.blocked)
    ? "blocked"
    : organizeWorkflowCurrent > 1
      ? "done"
      : "current";
  const organizeDryRunWorkflowState: WorkflowStepState = organizeWorkflowCurrent > 2 ? "done" : organizeWorkflowCurrent === 2 ? "current" : "pending";
  const organizeConfirmWorkflowState: WorkflowStepState = organizeWorkflowCurrent === 3 ? "current" : "pending";
  const dryRunError = Boolean(
    plan
      && dryRunErrorOwner
      && dryRunErrorOwner.planId === plan.id
      && dryRunErrorOwner.planRevision === plan.revision
      && !dryRun
  );
  const planToOpen = plans.find((item) => !isHistoricalOrganizationPlan(item.status)) ?? null;
  const hasOnlyHistoricalPlans = plans.length > 0 && plans.every((item) => isHistoricalOrganizationPlan(item.status));
  const canCreatePlan = plans.every((item) => isHistoricalOrganizationPlan(item.status))
    && (!plan || isHistoricalOrganizationPlan(plan.status));
  const showCreatePlanState = planListState === "loaded" && canCreatePlan;
  const openPlanSelectionId = openPlanErrorPlanId ?? planToOpen?.id ?? plans[0]?.id ?? "";
  const emptyPlanTitle = t("organizeNoPlanTitle");
  const emptyPlanDescription = t("organizeNoPlanDescription");
  const historicalPlanTitle = t("organizeCreateAnotherPlanTitle");
  const historicalPlanDescription = t("organizeCreateAnotherPlanDescription");

  const virtualizer = useVirtualizer({
    count: visibleGroups.length,
    getScrollElement: () => groupListRef.current,
    estimateSize: () => GROUP_ROW_HEIGHT,
    overscan: 6
  });
  const virtualRows = virtualizer.getVirtualItems();
  const mountedActiveId = virtualRows.some((row) => visibleGroups[row.index]?.groupId === activeGroupId)
    ? `organization-group-${activeGroupId}`
    : undefined;

  const loadGroupItems = useCallback(async (
    requestedPlanId: string,
    requestedPlanRevision: number,
    groupId: string,
    requestedProjectionFingerprint: string,
    cursor: string | null = null,
    append = false
  ) => {
    const epoch = ++groupRequestEpoch.current;
    const ownsRequest = () => {
      const currentPlan = useOrganizationPlanStore.getState().activePlan;
      return epoch === groupRequestEpoch.current
        && currentPlan?.id === requestedPlanId
        && currentPlan.revision === requestedPlanRevision;
    };
    const matchesPage = (page: { planId: string; groupId: string; planRevision: number; projectionFingerprint: string }) => {
      return page.planId === requestedPlanId
        && page.groupId === groupId
        && page.planRevision === requestedPlanRevision
        && page.projectionFingerprint === requestedProjectionFingerprint;
    };
    if (!ownsRequest()) return;
    if (!append) {
      setGroupItems([]);
      setGroupItemsCursor(null);
      setGroupItemsHasMore(false);
      setActiveItemId(null);
      setEditingItemId(null);
      setEditedName("");
      setEditError(null);
    }
    setGroupItemsLoading(true);
    setGroupItemsError(null);
    try {
      const page = await tauriApi.queryOrganizationPlanGroupItems({
        planId: requestedPlanId,
        groupId,
        cursor,
        expectedProjectionFingerprint: requestedProjectionFingerprint,
        pageSize: GROUP_PAGE_SIZE
      });
      if (!ownsRequest()) return;
      if (!matchesPage(page)) {
        setGroupItemsLoading(false);
        setGroupItemsCursor(null);
        setGroupItemsHasMore(false);
        setGroupItemsError(t("organizeLoadFailedDesc"));
        return;
      }
      setGroupItems((current) => append ? [...current, ...page.items] : page.items);
      setGroupItemsCursor(page.nextCursor);
      setGroupItemsHasMore(page.hasMore);
      setActiveItemId((current) => {
        if (append) return current ?? page.items[0]?.id ?? null;
        return page.items.some((item) => item.id === current) ? current : page.items[0]?.id ?? null;
      });
      setGroupItemsLoading(false);
    } catch (error) {
      if (!ownsRequest()) return;
      setGroupItemsLoading(false);
      setGroupItemsError(t("organizeLoadFailedDesc"));
      if (readableError(error).includes("organization_group_projection_changed")) {
        await openPlan(requestedPlanId);
      }
    }
  }, [openPlan, t]);

  useEffect(() => {
    if (!activeGroup) {
      groupRequestEpoch.current += 1;
      setGroupItems([]);
      setGroupItemsCursor(null);
      setGroupItemsHasMore(false);
      setGroupItemsLoading(false);
      setGroupItemsError(null);
      setActiveItemId(null);
      setEditingItemId(null);
      setEditedName("");
      setEditError(null);
      return;
    }
    if (!plan) return;
    void loadGroupItems(plan.id, plan.revision, activeGroup.groupId, activeGroup.projectionFingerprint).catch(() => undefined);
  }, [activeGroup?.groupId, activeGroup?.projectionFingerprint, loadGroupItems, plan?.id, plan?.revision]);

  useEffect(() => {
    if (activeGroupId && !groups.some((group) => group.groupId === activeGroupId)) {
      activeGroupIdRef.current = null;
      setActiveGroupId(null);
    }
  }, [activeGroupId, groups]);

  useEffect(() => {
    if (activeGroupId && !visibleGroups.some((group) => group.groupId === activeGroupId)) {
      activeGroupIdRef.current = null;
      setActiveGroupId(null);
    }
  }, [activeGroupId, visibleGroups]);

  useEffect(() => {
    if (visibleGroups.length || !groupHasMore || !groupNextCursor || isLoading) return;
    void loadNextGroupPage().catch(() => undefined);
  }, [groupHasMore, groupNextCursor, isLoading, loadNextGroupPage, visibleGroups.length]);

  useEffect(() => {
    const last = virtualRows.at(-1);
    if (last && groupHasMore && groupNextCursor && last.index >= visibleGroups.length - 8 && !isLoading) {
      void loadNextGroupPage().catch(() => undefined);
    }
  }, [groupHasMore, groupNextCursor, isLoading, loadNextGroupPage, visibleGroups.length, virtualRows]);

  function planSource(): { source: LibrarySelectionV1; expectedCount: number } | null {
    if (librarySelection?.kind === "explicit") {
      return { source: librarySelection, expectedCount: librarySelection.fileIds.length };
    }
    if (librarySelection?.kind === "all_matching" && totalCount !== null) {
      return { source: librarySelection, expectedCount: totalCount };
    }
    if (queryFingerprint && querySnapshotRevision !== null && totalCount !== null) {
      return {
        source: {
          kind: "all_matching",
          query: querySpec,
          queryFingerprint,
          snapshotRevision: querySnapshotRevision,
          excludedFileIds: []
        },
        expectedCount: totalCount
      };
    }
    return null;
  }

  async function handleCreatePlan() {
    if (oneOff.folderPath.trim()) { await oneOff.create(planTitle); return; }
    const source = planSource();
    if (!source) {
      setView("library");
      return;
    }
    try {
      const result = await createPlan(source.source, source.expectedCount, planTitle);
      if (result.applied) setPlanTitle("");
    } catch (error) {
      if (useOrganizationPlanStore.getState().activePlan && plan) setReviewActionError(organizeActionError(error, t));
    }
  }

  function handleGroupKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (!visibleGroups.length || event.ctrlKey || event.metaKey || event.altKey) return;
    const index = Math.max(0, visibleGroups.findIndex((group) => group.groupId === activeGroupId));
    let nextIndex = index;
    if (event.key === "ArrowDown") nextIndex = Math.min(visibleGroups.length - 1, index + 1);
    else if (event.key === "ArrowUp") nextIndex = Math.max(0, index - 1);
    else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = visibleGroups.length - 1;
    else if (event.key === "PageDown") nextIndex = Math.min(visibleGroups.length - 1, index + 5);
    else if (event.key === "PageUp") nextIndex = Math.max(0, index - 5);
    else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      activeGroupIdRef.current = visibleGroups[index]?.groupId ?? null;
      setActiveGroupId(visibleGroups[index]?.groupId ?? null);
      return;
    } else return;
    event.preventDefault();
    const next = visibleGroups[nextIndex];
    activeGroupIdRef.current = next.groupId;
    setActiveGroupId(next.groupId);
    virtualizer.scrollToIndex(nextIndex, { align: "auto" });
  }

  async function handleGroupDecision(group: OrganizationPlanGroupSummary, decision: "accepted" | "kept" | "undecided", confirmed = false) {
    const requestedPlan = useOrganizationPlanStore.getState().activePlan;
    if (!canReview || !requestedPlan || group.planId !== requestedPlan.id || group.revision !== requestedPlan.revision) return;
    const available = decision === "accepted"
      ? group.groupActions.canAcceptAll
      : decision === "kept"
        ? group.groupActions.canKeepAll
        : group.groupActions.canClearAll;
    if (!available) return;
    if (decision === "accepted" && group.readiness === "requires-decision" && !confirmed && !(requestedPlan.sourceKind === "one_off" && group.riskLevel === "Normal" && group.confidenceBand === "high" && group.reviewReasonCounts.every(({ reason }) => reason === "target_directory_creation"))) {
      setConfirmGroupAcceptance({ planId: requestedPlan.id, planRevision: requestedPlan.revision, groupId: group.groupId, group });
      return;
    }
    setReviewActionError(null);
    setReviewActionNeedsRefresh(false);
    try {
      const result = await updateGroupDecision(group, decision);
      if (result && !result.applied) return;
    } catch (error) {
      const currentPlan = useOrganizationPlanStore.getState().activePlan;
      if (currentPlan?.id !== requestedPlan.id || currentPlan.revision !== requestedPlan.revision) return;
      setReviewActionError(organizeActionError(error, t));
      setReviewActionNeedsRefresh(isOrganizationGroupChangedError(error));
    }
  }

  async function handleItemDecision(item: OrganizationPlanItem, decision: "accepted" | "kept" | "edited" | "undecided", name?: string): Promise<boolean> {
    const requestedPlan = useOrganizationPlanStore.getState().activePlan;
    const requestedGroupId = activeGroupIdRef.current;
    if (!canReview || !requestedPlan || item.planId !== requestedPlan.id || !requestedGroupId) return false;
    setEditError(null);
    try {
      const result = await updateDecision(item, decision, name);
      if (result && !result.applied) return false;
      const current = useOrganizationPlanStore.getState();
      if (current.activePlan?.id !== requestedPlan.id || activeGroupIdRef.current !== requestedGroupId) return false;
      const currentGroup = current.groups.find((group) => group.groupId === requestedGroupId);
      if (!currentGroup) return false;
      await loadGroupItems(current.activePlan.id, current.activePlan.revision, requestedGroupId, currentGroup.projectionFingerprint);
      return true;
    } catch (error) {
      const currentPlan = useOrganizationPlanStore.getState().activePlan;
      if (currentPlan?.id !== requestedPlan.id || activeGroupIdRef.current !== requestedGroupId) return false;
      setEditError(organizeActionError(error, t));
      return false;
    }
  }

  function requestItemAcceptance(item: OrganizationPlanItem) {
    if (!canReview || !item.availableActions.includes("accept_suggestion")) return;
    setEditError(null);
    if (item.validity === "needs_review") {
      const currentPlan = useOrganizationPlanStore.getState().activePlan;
      if (currentPlan) setConfirmItemAcceptance({ planId: currentPlan.id, planRevision: currentPlan.revision, itemId: item.id, item });
      return;
    }
    void handleItemDecision(item, "accepted").catch(() => undefined);
  }

  async function saveEditedName() {
    if (!activeItem) return;
    const requestedPlanId = plan?.id;
    const requestedItemId = activeItem.id;
    const validation = validateOrganizeFileNameForOriginal(activeItem.sourceNameSnapshot, editedName);
    if (validation) {
      setEditError(nameErrorCopy(validation, t));
      return;
    }
    if (await handleItemDecision(activeItem, "edited", editedName.trim())
      && useOrganizationPlanStore.getState().activePlan?.id === requestedPlanId
      && activeItemIdRef.current === requestedItemId) setEditingItemId(null);
  }

  async function reviewExecution() {
    const requestedPlan = useOrganizationPlanStore.getState().activePlan;
    const currentDryRun = useOrganizationPlanStore.getState().dryRun;
    if (!requestedPlan) return;
    if (currentDryRun) {
      setExecutionConfirmation({
        planId: requestedPlan.id,
        planRevision: requestedPlan.revision,
        dryRunFingerprint: currentDryRun.dryRunFingerprint,
        batch: organizationExecutionBatchSummary(currentDryRun.executableCount, currentDryRun.executionBatchLimit)
      });
      return;
    }
    setConfirmedExecutionBatch(null);
    setDryRunErrorOwner(null);
    try {
      const result = await createDryRun();
      if (!result.applied) return;
      setDryRunErrorOwner(null);
    } catch (error) {
      const currentPlan = useOrganizationPlanStore.getState().activePlan;
      if (currentPlan?.id === requestedPlan.id && currentPlan.revision === requestedPlan.revision) {
        setDryRunErrorOwner({ planId: requestedPlan.id, planRevision: requestedPlan.revision });
        setReviewActionError(organizeActionError(error, t));
      }
    }
  }

  async function handleRefreshPlan() {
    try {
      await refreshPlan();
    } catch (error) {
      if (useOrganizationPlanStore.getState().activePlan?.id === plan?.id) setReviewActionError(organizeActionError(error, t));
    }
  }

  async function handleAnalyzeMissing() {
    if (plan?.sourceKind === "one_off") {
      await oneOff.analyze();
      const summary = useOrganizationPlanStore.getState().activePlan?.effectiveSummary;
      setActiveTab(summary?.pendingReview ? "decision" : summary?.blocked ? "blocked" : "plan");
      return;
    }
    try {
      await analyzeMissing();
    } catch (error) {
      if (useOrganizationPlanStore.getState().activePlan?.id === plan?.id) setReviewActionError(organizeActionError(error, t));
    }
  }

  async function handleCancelPlan() {
    const currentPlan = useOrganizationPlanStore.getState().activePlan;
    if (!currentPlan || !["draft", "building", "ready", "stale"].includes(currentPlan.status)) return;
    try {
      await cancelPlan();
    } catch (error) {
      if (useOrganizationPlanStore.getState().activePlan?.id === plan?.id) setReviewActionError(organizeActionError(error, t));
    }
  }

  async function handleExecuteDryRun(confirmation: ExecutionConfirmation) {
    if (mutationUnavailable) return;
    const current = useOrganizationPlanStore.getState();
    if (current.activePlan?.id !== confirmation.planId
      || current.activePlan.revision !== confirmation.planRevision
      || current.dryRun?.dryRunFingerprint !== confirmation.dryRunFingerprint) return;
    try {
      const result = await executeDryRun();
      if (result && !result.applied) return;
    } catch (error) {
      const latest = useOrganizationPlanStore.getState().activePlan;
      if (latest?.id === confirmation.planId) setReviewActionError(organizeActionError(error, t));
    }
  }

  function openGroup(group: OrganizationPlanGroupSummary) {
    setEditedGroupFolder((group.targetDirectory ?? "").replace(/\\/g, "/").split("/").filter(Boolean).at(-1) ?? "");
    groupRequestEpoch.current += 1;
    setGroupItems([]);
    setGroupItemsCursor(null);
    setGroupItemsHasMore(false);
    setGroupItemsLoading(false);
    setActiveItemId(null);
    setEditingItemId(null);
    setEditedName("");
    activeGroupIdRef.current = group.groupId;
    setActiveGroupId(group.groupId);
    setEditError(null);
    setGroupItemsError(null);
  }

  function openHistory() {
    requestOrganizationHistoryBatch(executionResult?.operationBatchId ?? plan?.activeOperationBatchId ?? null);
    setView("restore");
  }

  return (
    <div className={cn(pageFrame, "gap-3") }>
      <AIReadinessNotice
        feature="organize"
        sourceMode={!plan || plan.sourceKind === "one_off" ? "one_off" : "managed"}
        readiness={aiReadiness}
        loading={aiReadinessLoading}
        error={aiReadinessError}
        planReadiness={organizationPlanReadiness(plan)}
        t={t}
        setView={setView}
        onRetry={() => void refreshAiReadiness().catch(() => undefined)}
      />
      {oneOff.error ? <NoticeBanner tone="error" title={t("organizeOneOffFailed")}>{oneOff.error}</NoticeBanner> : null}
      {oneOff.reading ? <div role="status">{t("organizeOneOffReading")} <Button variant="ghost" onClick={oneOff.cancel}>{t("fmCancel")}</Button></div> : null}
      {plan?.sourceKind === "one_off" && needsAnalysisCount > 0 ? <OneOffOrganizeConsent consent={oneOff.consent} busy={isMutating} providerReady={canGenerateSemanticProposals} onConsent={oneOff.setConsent} onAnalyze={() => void handleAnalyzeMissing()} t={t} /> : null}
      {oneOff.progress ? <div role="status" className="flex items-center gap-3"><span>{t("organizeOneOffProgress").replace("{done}", String(oneOff.progress.done)).replace("{total}", String(oneOff.progress.total))}</span><Button variant="secondary" onClick={oneOff.cancel}>{t("organizeOneOffStop")}</Button></div> : null}
      {isExecutionInFlight ? <div role="status">{t("organizeOneOffExecuting")} {operationProgress ? `${operationProgress.processed} / ${operationProgress.total}` : ""} <Button variant="secondary" onClick={() => void tauriApi.cancelOperations().catch(() => undefined)}>{t("fmCancel")}</Button></div> : null}
      {!plan && (planListState === "idle" || planListState === "loading") ? (
        <section className="grid min-h-0 flex-1 place-items-center">
          <div className="w-full max-w-xl">
            <DurableTaskStatus state="running" title={t("organizePlanListLoading")} description={t("organizePlanListLoadingDesc")} />
          </div>
        </section>
      ) : null}

      {!plan && planListState === "failed" ? (
        <section className="grid min-h-0 flex-1 place-items-center">
          <div className="w-full max-w-xl">
            <StateBlock
              tone="error"
              title={t("organizePlanListFailedTitle")}
              description={t("organizePlanListFailedDesc")}
              primaryAction={<Button variant="secondary" onClick={() => void loadPlans().catch(() => undefined)}>{t("organizePlanListRetry")}</Button>}
            />
          </div>
        </section>
      ) : null}

      {!plan && planListState === "loaded" && plans.length > 0 && activePlanState === "opening" ? (
        <section className="grid min-h-0 flex-1 place-items-center">
          <div className="w-full max-w-xl">
            <DurableTaskStatus state="running" title={t("organizePlanOpening")} description={t("organizePlanOpeningDesc")} />
            {plans.length > 1 ? <label className="mt-4 grid gap-1 text-sm text-[var(--zc-text-secondary)]" htmlFor="organization-plan-open-selector"><span>{t("organizePlanSelectorLabel")}</span><select id="organization-plan-open-selector" className={cn(inputSurface, "min-h-[var(--zc-control-height-default)] px-3 text-sm")} value={openPlanSelectionId} onChange={(event) => void openPlan(event.target.value).catch(() => undefined)} disabled={isExecutionInFlight}>{plans.map((item) => <option key={item.id} value={item.id}>{item.title}</option>)}</select></label> : null}
          </div>
        </section>
      ) : null}

      {!plan && planListState === "loaded" && plans.length > 0 && activePlanState === "failed" ? (
        <section className="grid min-h-0 flex-1 place-items-center">
          <div className="w-full max-w-xl">
            {plans.length > 1 ? <label className="mb-4 grid gap-1 text-sm text-[var(--zc-text-secondary)]" htmlFor="organization-plan-open-selector"><span>{t("organizePlanSelectorLabel")}</span><select id="organization-plan-open-selector" className={cn(inputSurface, "min-h-[var(--zc-control-height-default)] px-3 text-sm")} value={openPlanSelectionId} onChange={(event) => void openPlan(event.target.value).catch(() => undefined)} disabled={isExecutionInFlight}>{plans.map((item) => <option key={item.id} value={item.id}>{item.title}</option>)}</select></label> : null}
            <StateBlock
              tone="error"
              title={t("organizePlanOpenFailedTitle")}
              description={t("organizePlanOpenFailedDesc")}
              primaryAction={<Button variant="secondary" onClick={() => void openPlan(openPlanSelectionId).catch(() => undefined)}>{t("organizePlanOpenRetry")}</Button>}
            />
          </div>
        </section>
      ) : null}

      {showCreatePlanState ? (
        <section className="grid min-h-0 flex-1 place-items-center">
          <div className="grid w-full max-w-xl gap-4 rounded-[var(--zc-radius-panel)] border border-[var(--zc-border)] bg-[var(--zc-surface)] p-6 shadow-[var(--zc-shadow-soft)]">
            <div>
              <h2 className="text-lg font-semibold text-[var(--zc-text-primary)]">{hasOnlyHistoricalPlans ? historicalPlanTitle : emptyPlanTitle}</h2>
              <p className="mt-1 text-sm leading-6 text-[var(--zc-text-secondary)]">{hasOnlyHistoricalPlans ? historicalPlanDescription : emptyPlanDescription}</p>
            </div>
            <div className="grid gap-2">
              <label htmlFor="organization-folder">{t("organizeOneOffFolder")}</label>
              <div className="flex gap-2"><input id="organization-folder" className={cn(inputSurface, "min-w-0 flex-1 px-3")} value={oneOff.folderPath} onChange={(event) => oneOff.setFolderPath(event.target.value)} placeholder={t("organizeOneOffFolderPlaceholder")} disabled={isMutating} /><Button variant="secondary" disabled={isMutating} onClick={() => void oneOff.chooseFolder()}>{t("browseChooseFolderTitle")}</Button></div>
              <p className="text-xs text-[var(--zc-text-secondary)]">{t("organizeOneOffBoundary")}</p>
            </div>
            <div className="grid gap-2">
              <label className="text-sm font-medium text-[var(--zc-text-secondary)]" htmlFor="organization-plan-title">{t("organizePlanTitleLabel")}</label>
              <input id="organization-plan-title" className={cn(inputSurface, "min-h-[var(--zc-control-height-default)] px-3 text-sm")} value={planTitle} onChange={(event) => setPlanTitle(event.target.value)} placeholder={t("organizePlanTitlePlaceholder")} />
            </div>
            {createPlanError ? <NoticeBanner tone="error" title={t("organizeCreatePlanFailedTitle")}>{t("organizeCreatePlanFailedDesc")}</NoticeBanner> : null}
            <div className="flex flex-wrap gap-2">
              <Button variant="primary" disabled={isPlanListLoading || isLoading || isMutating || planListState !== "loaded" || (!oneOff.folderPath.trim() && !planSource())} onClick={() => void handleCreatePlan().catch(() => undefined)}><Plus size={15} aria-hidden="true" />{oneOff.folderPath.trim() ? t("organizeOneOffRead") : t("organizeCreatePlanAction")}</Button>
              <Button variant="secondary" onClick={() => setView("library")}>{t("fileLibrary")}</Button>
            </div>
          </div>
        </section>
      ) : null}

      {plan ? (
        <>
          <section className="grid shrink-0 gap-3 rounded-[var(--zc-radius-panel)] border border-[var(--zc-border)] bg-[var(--zc-surface)] p-3 shadow-[var(--zc-shadow-soft)]" data-organize-plan-header>
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="min-w-0">
                <label className="sr-only" htmlFor="organization-plan-selector">{t("organizePlanSelectorLabel")}</label>
                <select id="organization-plan-selector" className={cn(inputSurface, "min-h-[var(--zc-control-height-compact)] max-w-full min-w-56 px-2 text-sm")} value={plan.id} onChange={(event) => void openPlan(event.target.value).catch(() => undefined)} aria-label={t("organizePlanSelectorLabel")} disabled={isExecutionInFlight}>
                  {plans.map((item) => <option key={item.id} value={item.id}>{item.title}</option>)}
                </select>
                <p className="mt-1 text-xs text-[var(--zc-text-secondary)]">{t("organizePlanStatusLine").replace("{status}", planStatusLabel(plan.status, t)).replace("{count}", plan.materializedCount.toLocaleString())}</p>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <details className="relative">
                  <summary className={cn(buttonGhost, "cursor-pointer list-none")}>{t("organizePlanActions")} <MoreHorizontal size={14} aria-hidden="true" /></summary>
                  <div className="absolute right-0 z-20 mt-1 grid min-w-52 gap-1 rounded-[var(--zc-radius-field)] border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] p-1 shadow-[var(--zc-shadow-menu)]" role="menu">
                    <Button variant="ghost" size="compact" className="justify-start" disabled={isMutating || !["stale", "ready", "partially_completed"].includes(plan.status)} onClick={() => void handleRefreshPlan().catch(() => undefined)}><RefreshCw size={14} aria-hidden="true" />{t("organizePlanRefresh")}</Button>
                    <Button variant="ghost" size="compact" className="justify-start" disabled={isMutating || !needsAnalysisCount || !canGenerateSemanticProposals || (plan?.sourceKind === "one_off" && !oneOff.consent)} title={needsAnalysisCount && !canGenerateSemanticProposals ? t(plan?.sourceKind === "one_off" ? "organizeOneOffProviderError" : "organizeReadinessAnalyzeBlocked") : undefined} onClick={() => void handleAnalyzeMissing().catch(() => undefined)}><Sparkles size={14} aria-hidden="true" />{t("organizePlanAnalyze")}</Button>
                    <Button variant="ghost" size="compact" className="justify-start" disabled={isMutating || !canCancel} onClick={() => void handleCancelPlan().catch(() => undefined)}><X size={14} aria-hidden="true" />{t("organizePlanCancel")}</Button>
                  </div>
                </details>
                <Button variant="secondary" size="compact" onClick={openHistory}><History size={14} aria-hidden="true" />{t("organizeViewHistory")}</Button>
              </div>
            </div>
            <MetricStrip
              ariaLabel={t("organizePlanMetricLabel")}
              density="compact"
              items={[
                { label: t("organizePlanMetricFiles"), value: plan.materializedCount.toLocaleString() },
                { label: t("organizePlanMetricAccepted"), value: (plan.summary.accepted + plan.summary.edited).toLocaleString(), tone: "green" },
                { label: t("organizePlanMetricReview"), value: (effectiveSummary?.pendingReview ?? 0).toLocaleString(), tone: "amber" },
                { label: t("organizePlanMetricBlocked"), value: (effectiveSummary?.blocked ?? 0).toLocaleString(), tone: "red" }
              ]}
            />
          </section>

          {plan.sourceKind !== "one_off" ? <WorkflowSteps
            label={t("organizeWorkflowLabel")}
            className="shrink-0"
            steps={[
              { label: t("organizeWorkflowSuggestion"), detail: replaceCopy(t("organizeWorkflowPlanDetail"), { count: plan.materializedCount }), state: "done", stateLabel: t("workflowStateDone") },
              { label: t("organizeWorkflowReview"), detail: t("organizeWorkflowReviewDetail"), state: organizeReviewWorkflowState, stateLabel: workflowStateLabel(organizeReviewWorkflowState, t) },
              { label: t("organizeWorkflowDryRun"), detail: t("organizeWorkflowDryRunDetail"), state: organizeDryRunWorkflowState, stateLabel: workflowStateLabel(organizeDryRunWorkflowState, t) },
              { label: t("organizeWorkflowConfirm"), detail: t("organizeWorkflowConfirmDetail"), state: organizeConfirmWorkflowState, stateLabel: workflowStateLabel(organizeConfirmWorkflowState, t) }
            ]}
          /> : null}
          {plan.sourceKind !== "one_off" ? <NoticeBanner tone="info" title={t("organizeWorkflowBoundaryTitle")}>
            {t("organizeWorkflowBoundaryDesc")}
          </NoticeBanner> : null}
          {plan.sourceKind !== "one_off" && effectiveSummary?.blocked ? (
            <NoticeBanner tone="warning" title={replaceCopy(t("organizeBlockedSummary"), { count: effectiveSummary.blocked })}>
              {t("organizePreviewUnavailableDesc")}
            </NoticeBanner>
          ) : null}
          {planListState === "failed" ? <NoticeBanner tone="warning" title={t("organizePlanListFailedTitle")} action={<Button variant="secondary" size="compact" onClick={() => void loadPlans().catch(() => undefined)}>{t("organizePlanListRetry")}</Button>}>{t("organizePlanListFailedDesc")}</NoticeBanner> : null}
          {error ? <NoticeBanner tone="error" title={t("organizeLoadFailedTitle")} action={<Button variant="secondary" size="compact" onClick={() => void openPlan(plan.id).catch(() => undefined)}>{t("organizePlanRefresh")}</Button>}>{t("organizeLoadFailedDesc")}</NoticeBanner> : null}
          {reviewActionError ? <NoticeBanner tone="warning" title={t("organizeGroupActionFailed")} action={reviewActionNeedsRefresh ? <Button variant="secondary" size="compact" onClick={() => { setReviewActionError(null); setReviewActionNeedsRefresh(false); void handleRefreshPlan().catch(() => undefined); }}>{t("organizePlanRefresh")}</Button> : <Button variant="ghost" size="compact" onClick={() => setReviewActionError(null)}>{t("close")}</Button>}>{reviewActionError}</NoticeBanner> : null}

          <div className="grid min-h-0 flex-1 gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(220px,280px)]" data-organize-plan-shell>
            <div className="grid min-h-0 gap-3">
              <SegmentedControl
                value={activeTab}
                ariaLabel={t("organizePlanTabsLabel")}
                onChange={setActiveTab}
                options={[
                  { value: "plan", label: t("organizePlanTab") },
                  { value: "decision", label: t("organizeNeedsDecisionTab") },
                  { value: "blocked", label: t("organizeCannotProcessTab") }
                ]}
              />

              <section className="min-h-0 flex-1 overflow-hidden rounded-[var(--zc-radius-panel)] border border-[var(--zc-border)] bg-[var(--zc-surface)] max-[1100px]:min-h-[320px]" data-organize-groups>
                {isLoading && !groups.length ? <DurableTaskStatus state="running" title={t("organizeGroupLoading")} description={t("organizeLoadingSuggestionsDesc")} density="compact" /> : null}
                {!isLoading && !visibleGroups.length && groupHasMore && groupNextCursor ? <StateBlock tone="info" title={t("organizeGroupLoading")} description={t("organizeLoadingSuggestionsDesc")} primaryAction={<Button variant="secondary" size="compact" onClick={() => void loadNextGroupPage().catch(() => undefined)}>{t("organizeGroupLoadMore")}</Button>} density="compact" /> : null}
                {!isLoading && !visibleGroups.length && (!groupHasMore || !groupNextCursor) ? <StateBlock tone={activeTab === "blocked" ? "info" : "neutral"} title={emptyTabTitle(activeTab, t)} description={emptyTabDescription(activeTab, t)} density="compact" /> : null}
                {visibleGroups.length ? (
                  <div ref={groupListRef} className="h-full overflow-auto outline-none" role="listbox" tabIndex={0} aria-label={t("organizeGroupListLabel")} aria-activedescendant={mountedActiveId} onKeyDown={handleGroupKeyDown}>
                    <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
                      {virtualRows.map((virtualRow) => {
                        const group = visibleGroups[virtualRow.index];
                        const active = group.groupId === activeGroupId;
                        return (
                          <div
                            key={group.groupId}
                            id={`organization-group-${group.groupId}`}
                            role="option"
                            aria-selected={active}
                            data-organize-group-row={group.groupId}
                            className={cn("absolute left-0 top-0 grid w-full gap-2 border-b border-[var(--zc-divider)] px-4 py-3 text-left transition-[background,border-color]", active && selectedFocusSurface)}
                            style={{ minHeight: virtualRow.size, transform: `translateY(${virtualRow.start}px)` }}
                            onClick={() => openGroup(group)}
                          >
                            <div className="flex min-w-0 items-start justify-between gap-3">
                              <div className="min-w-0">
                                <strong className="block truncate text-sm text-[var(--zc-text-primary)]">{group.targetDirectory ?? t("organizeGroupNoDestination")}</strong>
                                <p className="mt-1 truncate text-xs text-[var(--zc-text-secondary)]">{proposalKindLabel(group.proposalKind, t)} · {readinessLabel(group.readiness, t)} · {confidenceLabel(group.confidenceBand, t)}</p>
                              </div>
                              <ChevronRight size={16} className="mt-1 shrink-0 text-[var(--zc-text-tertiary)]" aria-hidden="true" />
                            </div>
                            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs tabular-nums text-[var(--zc-text-secondary)]">
                              <span>{t("organizeGroupFiles").replace("{count}", group.itemCount.toLocaleString())}</span>
                              <span>{t("organizeGroupBytes").replace("{size}", formatBytes(group.totalBytes))}</span>
                              <span>{riskLabel(group.riskLevel, t)}</span>
                              {group.acceptedCount ? <span>{t("organizeGroupAccepted").replace("{count}", group.acceptedCount.toLocaleString())}</span> : null}
                              {group.excludedCount ? <span>{t("organizeGroupExcluded").replace("{count}", group.excludedCount.toLocaleString())}</span> : null}
                              {group.staleCount || group.conflictCount ? <span className="text-[var(--zc-warning-text)]">{t("organizeGroupIssues").replace("{stale}", group.staleCount.toLocaleString()).replace("{conflicts}", group.conflictCount.toLocaleString())}</span> : null}
                            </div>
                            {group.sampleItems[0]?.reason ? <p className="truncate text-xs text-[var(--zc-text-secondary)]" title={group.sampleItems[0].reason}>{group.sampleItems[0].reason}</p> : null}
                            <div className="flex flex-wrap items-center justify-between gap-2">
                              <span className="min-w-0 truncate text-xs text-[var(--zc-text-tertiary)]">{groupReason(group, t)}</span>
                              <div className="flex shrink-0 flex-wrap gap-2" onClick={(event) => event.stopPropagation()}>
                                {group.groupActions.canAcceptAll ? <Button variant="secondary" size="compact" disabled={isMutating || !canReview} onClick={() => void handleGroupDecision(group, "accepted").catch(() => undefined)}><Check size={13} aria-hidden="true" />{t("organizeGroupInclude")}</Button> : null}
                                {group.groupActions.canKeepAll ? <Button variant="ghost" size="compact" disabled={isMutating || !canReview} onClick={() => void handleGroupDecision(group, "kept").catch(() => undefined)}><CircleMinus size={13} aria-hidden="true" />{t("organizeGroupKeep")}</Button> : null}
                                {group.groupActions.canClearAll ? <Button variant="ghost" size="compact" disabled={isMutating || !canReview} onClick={() => void handleGroupDecision(group, "undecided").catch(() => undefined)}><ListRestart size={13} aria-hidden="true" />{t("organizeGroupClear")}</Button> : null}
                                {groupDecisionState(group, t) ? <span className="self-center text-xs font-medium text-[var(--zc-text-secondary)]">{groupDecisionState(group, t)}</span> : null}
                                <Button variant="ghost" size="compact" onClick={() => openGroup(group)}>{t("organizeGroupReview")}</Button>
                              </div>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                ) : null}
              </section>
            </div>
            <aside className="grid content-start gap-3 rounded-[var(--zc-radius-panel)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-4" data-organize-execution-boundary>
              <div>
                <h2 className="text-sm font-semibold text-[var(--zc-text-primary)]">{t("organizeWorkflowBoundaryTitle")}</h2>
                <p className="mt-1 text-sm leading-6 text-[var(--zc-text-secondary)]">{t("organizeWorkflowBoundaryDesc")}</p>
              </div>
              <dl className="grid gap-2 border-t border-[var(--zc-divider)] pt-3 text-sm">
                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--zc-text-secondary)]">{t("organizePlanMetricFiles")}</dt><dd className="font-semibold tabular-nums text-[var(--zc-text-primary)]">{plan.materializedCount.toLocaleString()}</dd></div>
                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--zc-text-secondary)]">{t("organizePlanMetricAccepted")}</dt><dd className="font-semibold tabular-nums text-[var(--zc-success-text)]">{(plan.summary.accepted + plan.summary.edited).toLocaleString()}</dd></div>
                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--zc-text-secondary)]">{t("organizePlanMetricReview")}</dt><dd className="font-semibold tabular-nums text-[var(--zc-warning-text)]">{(effectiveSummary?.pendingReview ?? 0).toLocaleString()}</dd></div>
                <div className="flex items-center justify-between gap-3"><dt className="text-[var(--zc-text-secondary)]">{t("organizePlanMetricBlocked")}</dt><dd className="font-semibold tabular-nums text-[var(--zc-danger-text)]">{(effectiveSummary?.blocked ?? 0).toLocaleString()}</dd></div>
              </dl>
            </aside>
          </div>

          <footer className="sticky bottom-0 z-10 flex shrink-0 flex-wrap items-center justify-between gap-3 rounded-[var(--zc-radius-panel)] border border-[var(--zc-border)] bg-[var(--zc-surface-overlay)] px-4 py-3 shadow-[var(--zc-shadow-raised)]" data-organize-review-action>
            <span className="text-xs leading-5 text-[var(--zc-text-secondary)]">{t("organizeReviewExecutionHint")}</span>
            <Button variant="primary" disabled={isMutating || !canDryRun || (Boolean(dryRun) && Boolean(mutationUnavailable))} title={mutationUnavailable ? t("errorMacosFileMutationSourceBindingUnsupported") : undefined} onClick={() => void reviewExecution().catch(() => undefined)}><Play size={15} aria-hidden="true" />{dryRun ? t("organizeDryRunAction") : t("organizeReviewExecution")}</Button>
          </footer>

          {mutationUnavailable ? <NoticeBanner tone="warning">{t("errorMacosFileMutationSourceBindingUnsupported")}</NoticeBanner> : null}

          {dryRun ? (
            <DurableTaskStatus
              state="completed"
              title={t("organizeDryRunTitle")}
              description={replaceCopy(t("organizeDryRunDesc"), {
                executable: dryRun.executableCount.toLocaleString(),
                batch: dryRunBatch?.batchCount.toLocaleString() ?? "0",
                remaining: dryRunBatch?.remainingCount.toLocaleString() ?? "0",
                blocked: dryRun.blockedCount.toLocaleString(),
                stale: dryRun.staleCount.toLocaleString()
              })}
              action={<Button variant="secondary" onClick={() => void reviewExecution().catch(() => undefined)}>{t("organizeDryRunAction")}</Button>}
              density="compact"
            />
          ) : null}

          {dryRunError ? (
            <DurableTaskStatus
              state="failed"
              title={t("organizePreviewUnavailableTitle")}
              description={t("organizePreviewUnavailableDesc")}
              action={<Button variant="secondary" size="compact" onClick={() => { setDryRunErrorOwner(null); void reviewExecution().catch(() => undefined); }}>{t("organizePlanRefresh")}</Button>}
              density="compact"
            />
          ) : null}

          {executionResult ? <NoticeBanner tone={executionResult.succeededCount !== executionResult.attemptedCount ? "warning" : "success"} title={executionResult.succeededCount !== executionResult.attemptedCount ? t("organizeResultPartialTitle") : t("organizeResultSuccessTitle")} action={<Button variant="secondary" size="compact" onClick={openHistory}>{t("organizeViewHistory")}</Button>}>
            <span>{replaceCopy(t("organizeResultSummary"), { success: executionResult.succeededCount.toLocaleString(), skipped: executionResult.skippedCount.toLocaleString(), failed: executionResult.failedCount.toLocaleString() })}</span>
            {confirmedExecutionBatch?.batch.isBatched ? <span className="mt-1 block">{replaceCopy(t("organizeExecutionBatchResult"), { attempted: executionResult.attemptedCount.toLocaleString(), total: confirmedExecutionBatch.batch.batchCount.toLocaleString(), remaining: confirmedExecutionBatch.batch.remainingCount.toLocaleString() })}</span> : null}
          </NoticeBanner> : null}
        </>
      ) : null}

      <SideSheet
        open={Boolean(activeGroup)}
        title={activeGroup ? (activeGroup.targetDirectory ?? t("organizeGroupNoDestination")) : t("organizeGroupDetails")}
        description={activeGroup ? `${t("organizeGroupFiles").replace("{count}", activeGroup.itemCount.toLocaleString())} · ${proposalKindLabel(activeGroup.proposalKind, t)}` : undefined}
        closeLabel={t("close")}
        restoreFocus={() => groupListRef.current}
        onClose={() => { groupRequestEpoch.current += 1; activeGroupIdRef.current = null; setActiveGroupId(null); setGroupItems([]); setGroupItemsCursor(null); setGroupItemsHasMore(false); setGroupItemsLoading(false); setActiveItemId(null); setEditingItemId(null); setEditedName(""); setEditError(null); setGroupItemsError(null); }}
        footer={activeGroup ? (
          <div className="flex flex-wrap justify-end gap-2">
            {activeGroup.groupActions.canAcceptAll ? <Button variant="secondary" disabled={isMutating || !canReview} onClick={() => void handleGroupDecision(activeGroup, "accepted").catch(() => undefined)}><Check size={14} aria-hidden="true" />{t("organizeGroupInclude")}</Button> : null}
            {activeGroup.groupActions.canKeepAll ? <Button variant="ghost" disabled={isMutating || !canReview} onClick={() => void handleGroupDecision(activeGroup, "kept").catch(() => undefined)}><CircleMinus size={14} aria-hidden="true" />{t("organizeGroupKeep")}</Button> : null}
            {activeGroup.groupActions.canClearAll ? <Button variant="ghost" disabled={isMutating || !canReview} onClick={() => void handleGroupDecision(activeGroup, "undecided").catch(() => undefined)}><ListRestart size={14} aria-hidden="true" />{t("organizeGroupClear")}</Button> : null}
            {groupDecisionState(activeGroup, t) ? <span className="self-center text-xs font-medium text-[var(--zc-text-secondary)]">{groupDecisionState(activeGroup, t)}</span> : null}
          </div>
        ) : undefined}
      >
        {activeGroup ? (
          <div className="grid gap-4">
            <div className="grid gap-2 rounded-[var(--zc-radius-row)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3">
              <span className="text-xs font-semibold uppercase tracking-[0.1em] text-[var(--zc-text-tertiary)]">{t("organizeGroupDestination")}</span>
              <span className="break-all text-sm text-[var(--zc-text-primary)]">{activeGroup.targetDirectory ?? t("organizeGroupNoDestination")}</span>
              <span className="text-xs text-[var(--zc-text-secondary)]">{groupReason(activeGroup, t)}</span>
              {activeGroup.readiness === "requires-decision" ? <p className="text-xs leading-5 text-[var(--zc-warning-text)]">{t("organizeGroupDecisionHint")}</p> : null}
              <div className="flex flex-wrap gap-2 text-xs text-[var(--zc-text-secondary)]"><span>{riskLabel(activeGroup.riskLevel, t)}</span><span>{confidenceLabel(activeGroup.confidenceBand, t)}</span><span>{readinessLabel(activeGroup.readiness, t)}</span></div>
            </div>
            {plan?.sourceKind === "one_off" && ["move", "move_rename", "rename"].includes(activeGroup.proposalKind) && canReview ? <form className="grid gap-2" onSubmit={(event) => {
              event.preventDefault();
              void updateGroupDecision(activeGroup, "undecided", editedGroupFolder.trim()).catch((error) => setReviewActionError(organizeActionError(error, t)));
            }}><label htmlFor="organize-group-folder">{t("organizeOneOffGroupFolder")}</label><div className="flex gap-2"><input id="organize-group-folder" className={cn(inputSurface, "min-w-0 flex-1 px-2")} value={editedGroupFolder} onChange={(event) => setEditedGroupFolder(event.target.value)} disabled={isMutating} /><Button type="submit" variant="secondary" disabled={isMutating || !editedGroupFolder.trim()}>{t("organizeOneOffGroupApply")}</Button></div></form> : null}
            {groupItemsError ? <NoticeBanner tone="error" title={t("organizeLoadFailedTitle")}>{groupItemsError}</NoticeBanner> : null}
            {editError ? <NoticeBanner tone="warning" title={t("organizeGroupActionFailed")}>{editError}</NoticeBanner> : null}
            <section className="grid gap-2" aria-label={t("organizeGroupItemListLabel")}>
              <h3 className="text-sm font-semibold text-[var(--zc-text-primary)]">{t("organizeGroupSamples")}</h3>
              {groupItemsLoading && !groupItems.length ? <p className="text-sm text-[var(--zc-text-secondary)]">{t("organizeGroupLoading")}</p> : null}
              {!groupItemsLoading && !groupItems.length ? <p className="text-sm text-[var(--zc-text-secondary)]">{t("organizeGroupNoItems")}</p> : null}
              <div className="grid gap-2" role="listbox" aria-label={t("organizeGroupItemListLabel")}>
                {groupItems.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={cn("grid min-w-0 gap-1 rounded-[var(--zc-radius-row)] border border-[var(--zc-border)] bg-[var(--zc-surface)] p-3 text-left hover:border-[var(--zc-control-border-hover)] hover:bg-[var(--zc-surface-hover)]", item.id === activeItemId ? selectedFocusVisibleState : focusVisibleState, item.id === activeItemId && selectedSurface)}
                    role="option"
                    aria-selected={item.id === activeItemId}
                    onClick={() => { setActiveItemId(item.id); setEditingItemId(null); }}
                  >
                    <span className="flex min-w-0 items-center justify-between gap-2"><strong className="truncate text-sm text-[var(--zc-text-primary)]">{item.sourceNameSnapshot}</strong><span className="shrink-0 text-xs text-[var(--zc-text-secondary)]">{decisionLabel(item.decision, t)}</span></span>
                    <span className="truncate text-xs text-[var(--zc-text-tertiary)]">{item.sourcePathSnapshot}</span>
                    <span className="truncate text-xs text-[var(--zc-text-secondary)]">{t("organizeGroupItemTo")}: {item.proposedTargetPath}</span>
                  </button>
                ))}
              </div>
              {groupItemsHasMore && groupItemsCursor && plan && activeGroup ? <Button variant="ghost" size="compact" disabled={groupItemsLoading} onClick={() => void loadGroupItems(plan.id, plan.revision, activeGroup.groupId, activeGroup.projectionFingerprint, groupItemsCursor, true).catch(() => undefined)}>{groupItemsLoading ? <LoaderCircle size={14} className="animate-spin" aria-hidden="true" /> : null}{t("organizeGroupLoadMore")}</Button> : null}
            </section>
            {activeItem ? (
              <section className="grid gap-3 border-t border-[var(--zc-divider)] pt-4" aria-label={t("organizeGroupDetails")}>
                <div className="grid gap-2 text-sm"><div><span className="text-xs text-[var(--zc-text-tertiary)]">{t("organizeGroupItemFrom")}</span><p className="mt-1 break-all text-[var(--zc-text-secondary)]">{activeItem.sourcePathSnapshot}</p></div><div><span className="text-xs text-[var(--zc-text-tertiary)]">{t("organizeGroupItemTo")}</span><p className="mt-1 break-all text-[var(--zc-text-secondary)]">{activeItem.proposedTargetPath}</p></div></div>
                <div
                  className="grid gap-1 rounded-[var(--zc-radius-row)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3"
                  data-organize-semantic-explanation
                  data-semantic-explanation-current={activeItem.semanticExplanation ? "true" : "false"}
                  data-semantic-explanation-source={activeItem.semanticExplanation?.assessmentFingerprint.startsWith("browser-presentation-only:") ? "presentation-fixture" : activeItem.semanticExplanation ? "backend-authority" : "unavailable"}
                  data-assessment-fingerprint={activeItem.semanticExplanation?.assessmentFingerprint ?? undefined}
                >
                  <strong className="text-xs font-semibold text-[var(--zc-text-secondary)]">{t("organizeSemanticExplanation")}</strong>
                  <p className="break-words text-sm leading-5 text-[var(--zc-text-primary)]">{activeItem.semanticExplanation?.reason || t("organizeSemanticExplanationUnavailable")}</p>
                  {plan?.sourceKind === "one_off" ? (
                    <p className="text-xs leading-5 text-[var(--zc-text-secondary)]" data-one-off-provider-provenance>
                      {activeItem.semanticExplanation?.providerKind && activeItem.semanticExplanation.model
                        ? replaceCopy(t(activeItem.semanticExplanation.providerSettingsCurrent === true
                          ? "organizeSemanticProviderCurrent"
                          : activeItem.semanticExplanation.providerSettingsCurrent === false
                            ? "organizeSemanticProviderChanged"
                            : "organizeSemanticProviderUnknown"), {
                            provider: activeItem.semanticExplanation.providerKind === "ollama" ? "Ollama" : "OpenAI-compatible",
                            model: activeItem.semanticExplanation.model
                          })
                        : t("organizeSemanticProviderUnknown")}
                    </p>
                  ) : null}
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button
                    variant="secondary"
                    size="compact"
                    disabled={!canReview || !activeItem.availableActions.includes("accept_suggestion") || isMutating}
                    title={itemAcceptUnavailableReason(activeItem, t)}
                    onClick={() => requestItemAcceptance(activeItem)}
                  ><Check size={14} aria-hidden="true" />{activeItem.availableActions.includes("accept_suggestion") ? t("organizeGroupItemAccept") : t("organizeGroupItemAcceptUnavailable")}</Button>
                  <Button variant="ghost" size="compact" disabled={!canReview || !activeItem.availableActions.includes("keep") || isMutating} onClick={() => void handleItemDecision(activeItem, "kept").catch(() => undefined)}><CircleMinus size={14} aria-hidden="true" />{t("organizeGroupItemKeep")}</Button>
                  <Button variant="ghost" size="compact" disabled={!canReview || !activeItem.availableActions.includes("edit_name") || isMutating} onClick={() => { setEditedName(activeItem.editedName ?? activeItem.proposedName); setEditingItemId(activeItem.id); setEditError(null); }}><Edit3 size={14} aria-hidden="true" />{t("organizeGroupItemEdit")}</Button>
                  <Button variant="ghost" size="compact" disabled={!canReview || !activeItem.availableActions.includes("clear_decision") || isMutating} onClick={() => void handleItemDecision(activeItem, "undecided").catch(() => undefined)}><ListRestart size={14} aria-hidden="true" />{t("organizeGroupItemClear")}</Button>
                </div>
                {activeItem.reviewReasons.length ? <p className="text-xs leading-5 text-[var(--zc-warning-text)]">{t("organizeItemReviewReasonLabel")}: {activeItem.reviewReasons.map((reason) => reviewReasonLabel(reason, t)).join(" · ")}</p> : null}
                {editingItemId === activeItem.id ? <div className="grid gap-2 rounded-[var(--zc-radius-row)] border border-[var(--zc-border)] p-3"><label className="text-xs font-medium text-[var(--zc-text-secondary)]" htmlFor="organization-group-edited-name">{t("organizeEditTargetName")}</label><input id="organization-group-edited-name" className={cn(inputSurface, "min-h-[var(--zc-control-height-default)] px-2")} value={editedName} onChange={(event) => setEditedName(event.target.value)} autoFocus /><div className="flex flex-wrap gap-2"><Button variant="secondary" size="compact" onClick={() => void saveEditedName().catch(() => undefined)}>{t("save")}</Button><Button variant="ghost" size="compact" onClick={() => setEditingItemId(null)}>{t("cancel")}</Button></div></div> : null}
              </section>
            ) : null}
          </div>
        ) : null}
      </SideSheet>

      <ConfirmDialog
        open={Boolean(confirmItemAcceptance)}
        tone="warning"
        title={t("organizeItemAcceptReviewTitle")}
        description={confirmItemAcceptance ? replaceCopy(t("organizeItemAcceptReviewDesc"), {
          reason: confirmItemAcceptance.item.reviewReasons.map((reason) => reviewReasonLabel(reason, t)).join(" · ") || t("organizeReasonFromAnalysis"),
          target: confirmItemAcceptance.item.proposedTargetPath
        }) : ""}
        confirmLabel={t("organizeItemAcceptReviewConfirm")}
        cancelLabel={t("cancel")}
        isProcessing={isMutating}
        onCancel={() => setConfirmItemAcceptance(null)}
        onConfirm={() => {
          const confirmation = confirmItemAcceptance;
          setConfirmItemAcceptance(null);
          const current = useOrganizationPlanStore.getState();
          if (confirmation
            && current.activePlan?.id === confirmation.planId
            && current.activePlan.revision === confirmation.planRevision
            && activeGroupIdRef.current) void handleItemDecision(confirmation.item, "accepted").catch(() => undefined);
        }}
      />

      <ConfirmDialog
        open={Boolean(confirmGroupAcceptance)}
        tone="warning"
        title={t("organizeGroupAcceptReviewTitle")}
        description={confirmGroupAcceptance ? replaceCopy(t("organizeGroupAcceptReviewDesc"), {
          reason: confirmGroupAcceptance.group.reviewReasonCounts.map(({ reason, count }) => `${reviewReasonLabel(reason, t)} (${count})`).join(" · ") || t("organizeReasonFromAnalysis")
        }) : ""}
        confirmLabel={t("organizeGroupAcceptReviewConfirm")}
        cancelLabel={t("cancel")}
        isProcessing={isMutating}
        onCancel={() => setConfirmGroupAcceptance(null)}
        onConfirm={() => {
          const confirmation = confirmGroupAcceptance;
          setConfirmGroupAcceptance(null);
          const current = useOrganizationPlanStore.getState();
          if (confirmation
            && current.activePlan?.id === confirmation.planId
            && current.activePlan.revision === confirmation.planRevision) void handleGroupDecision(confirmation.group, "accepted", true).catch(() => undefined);
        }}
      />

      <ConfirmDialog
        open={Boolean(executionConfirmation)}
        tone="warning"
        title={dryRun?.items.some((item) => item.riskLevel !== "Normal" || item.requiresConfirmation) ? t("organizeExecuteRiskConfirmTitle") : t("organizeExecuteNormalConfirmTitle")}
        description={dryRun && dryRunBatch ? replaceCopy(t(dryRunBatch.isBatched ? "organizeExecuteCappedConfirmDesc" : "organizeExecuteConfirmDesc"), {
          count: dryRunBatch.batchCount.toLocaleString(),
          total: dryRun.executableCount.toLocaleString(),
          remaining: dryRunBatch.remainingCount.toLocaleString()
        }) : ""}
        confirmLabel={t("organizeExecuteConfirmAction").replace("{count}", dryRunBatch?.batchCount.toLocaleString() ?? "0")}
        cancelLabel={t("cancel")}
        isProcessing={isMutating}
        disabled={Boolean(mutationUnavailable)}
        onCancel={() => setExecutionConfirmation(null)}
        onConfirm={() => {
          const confirmation = executionConfirmation;
          const current = useOrganizationPlanStore.getState();
          if (!confirmation
            || current.activePlan?.id !== confirmation.planId
            || current.activePlan.revision !== confirmation.planRevision
            || current.dryRun?.dryRunFingerprint !== confirmation.dryRunFingerprint) {
            setExecutionConfirmation(null);
            return;
          }
          setConfirmedExecutionBatch(confirmation);
          setExecutionConfirmation(null);
          void handleExecuteDryRun(confirmation).catch(() => undefined);
        }}
      />
    </div>
  );
}

function planStatusLabel(status: OrganizationPlanStatus, t: Translator): string {
  if (status === "draft") return t("organizePlanStatusDraft");
  if (status === "building") return t("organizePlanStatusBuilding");
  if (status === "ready") return t("organizePlanStatusReady");
  if (status === "stale") return t("organizePlanStatusStale");
  if (status === "executing") return t("organizePlanStatusExecuting");
  if (status === "partially_completed") return t("organizePlanStatusPartial");
  if (status === "completed") return t("organizePlanStatusCompleted");
  if (status === "cancelled") return t("organizePlanStatusCancelled");
  if (status === "failed") return t("organizePlanStatusFailed");
  return t("organizePlanStatusUnknown");
}

function proposalKindLabel(kind: string, t: Translator): string {
  if (kind === "move") return t("organizeGroupProposalMove");
  if (kind === "rename") return t("organizeGroupProposalRename");
  if (kind === "move_rename") return t("organizeGroupProposalMoveRename");
  if (kind === "keep") return t("organizeGroupProposalKeep");
  if (kind === "blocked") return t("organizeGroupProposalBlocked");
  return t("organizeGroupProposalUnknown");
}

function readinessLabel(readiness: OrganizationPlanGroupSummary["readiness"], t: Translator): string {
  if (readiness === "ready") return t("organizeGroupReadinessReady");
  if (readiness === "requires-decision") return t("organizeGroupReadinessDecision");
  if (readiness === "reviewed") return t("organizeGroupReadinessReviewed");
  return t("organizeGroupReadinessBlocked");
}

function groupDecisionState(group: OrganizationPlanGroupSummary, t: Translator): string | null {
  if (group.proposalKind === "keep") return t("organizeGroupNoMove");
  if (group.itemCount > 0 && group.acceptedCount === group.itemCount && !group.groupActions.canAcceptAll) {
    return t("organizeGroupIncluded");
  }
  if (group.readiness === "reviewed" && !group.groupActions.canAcceptAll) {
    return t("organizeGroupReviewed");
  }
  return null;
}

function riskLabel(risk: string, t: Translator): string {
  if (risk === "Normal") return t("organizeRiskNormal");
  if (risk === "Sensitive") return t("organizeRiskSensitive");
  if (risk === "System") return t("organizeRiskSystem");
  if (risk === "Caution") return t("organizeRiskCaution");
  return t("organizeRiskUnknown");
}

function confidenceLabel(confidence: string, t: Translator): string {
  if (confidence === "high") return t("organizeConfidenceHigh");
  if (confidence === "medium") return t("organizeConfidenceMedium");
  if (confidence === "low") return t("organizeConfidenceLow");
  return t("organizeConfidenceMixed");
}

function decisionLabel(decision: OrganizationPlanItem["decision"], t: Translator): string {
  if (decision === "accepted") return t("organizeDecisionAccepted");
  if (decision === "kept") return t("organizeDecisionKept");
  if (decision === "edited") return t("organizeDecisionEdited");
  return t("organizeDecisionUndecided");
}

function groupReason(group: OrganizationPlanGroupSummary, t: Translator): string {
  if (group.reviewReasonCounts.length) {
    return group.reviewReasonCounts
      .slice(0, 3)
      .map(({ reason, count }) => `${reviewReasonLabel(reason, t)} (${count})`)
      .join(" · ");
  }
  if (group.readiness === "blocked" && group.proposalKind === "keep") return t("organizeGroupReasonAnalysis");
  if (group.readiness === "blocked") return t("organizeGroupReasonBlocked");
  return t("organizeReasonFromAnalysis");
}

function reviewReasonLabel(reason: string, t: Translator): string {
  const labels: Record<string, Parameters<Translator>[0]> = {
    low_confidence: "organizeReviewReasonLowConfidence",
    sensitive_file: "organizeReviewReasonSensitiveFile",
    non_normal_risk: "organizeReviewReasonNonNormalRisk",
    possible_duplicate: "organizeReviewReasonPossibleDuplicate",
    requires_confirmation: "organizeReviewReasonRequiresConfirmation",
    target_directory_creation: "organizeReviewReasonTargetDirectoryCreation",
    target_collision: "organizeReviewReasonTargetCollision",
    source_changed: "organizeReviewReasonSourceChanged",
    proposal_changed: "organizeReviewReasonProposalChanged",
    managed_scope_changed: "organizeReviewReasonManagedScopeChanged",
    missing_preview: "organizeReviewReasonMissingPreview",
    unsupported_operation: "organizeReviewReasonUnsupportedOperation",
    unsafe_filename: "organizeReviewReasonUnsafeFilename",
    extension_change_blocked: "organizeReviewReasonExtensionBlocked"
  };
  return t(labels[reason] ?? "organizeReviewReasonUnknown");
}

function itemAcceptUnavailableReason(item: OrganizationPlanItem, t: Translator): string {
  if (item.validity === "stale") return t("organizeItemAcceptUnavailableChanged");
  if (item.validity === "blocked") return t("organizeItemAcceptUnavailableBlocked");
  if (!item.authoritativePreviewId || !item.availableActions.includes("view_preview")) return t("organizeItemAcceptUnavailablePreview");
  if (item.availableActions.includes("accept_suggestion")) return t("organizeItemAcceptReviewHint");
  return t("organizeItemAcceptUnavailableState");
}

function organizeActionError(error: unknown, t: Translator): string {
  const message = readableError(error);
  if (message.includes("organization_item_accept_not_available")) return t("organizeItemAcceptUnavailableBackend");
  if (message.includes("organization_item_edit_not_available")) return t("organizeItemEditUnavailableBackend");
  if (message.includes("organization_group_action_not_available")) return t("organizeGroupActionUnavailable");
  if (message.includes("organization_group_changed")) return t("organizeGroupChanged");
  return t("organizeGroupActionFailed");
}

function isOrganizationGroupChangedError(error: unknown): boolean {
  const message = readableError(error);
  return message.includes("organization_group_changed");
}

function emptyTabTitle(tab: ReviewTab, t: Translator): string {
  if (tab === "decision") return t("organizeNoDecisionGroupsTitle");
  if (tab === "blocked") return t("organizeNoBlockedGroupsTitle");
  return t("organizeNoReadyGroupsTitle");
}

function emptyTabDescription(tab: ReviewTab, t: Translator): string {
  if (tab === "decision") return t("organizeNoDecisionGroupsDescription");
  if (tab === "blocked") return t("organizeNoBlockedGroupsDescription");
  return t("organizeNoReadyGroupsDescription");
}

function nameErrorCopy(error: "empty" | "reserved" | "unsafe" | "extension", t: Translator): string {
  if (error === "empty") return t("organizeNameErrorEmpty");
  if (error === "reserved") return t("organizeNameErrorReserved");
  if (error === "extension") return t("organizeNameErrorExtension");
  return t("organizeNameErrorUnsafe");
}

function workflowStateLabel(state: WorkflowStepState, t: Translator): string {
  if (state === "done") return t("workflowStateDone");
  if (state === "current") return t("workflowStateCurrent");
  if (state === "blocked") return t("workflowStateBlocked");
  return t("workflowStatePending");
}

function replaceCopy(template: string, values: Record<string, string | number>): string {
  return Object.entries(values).reduce((copy, [key, value]) => copy.replaceAll(`{${key}}`, String(value)), template);
}
