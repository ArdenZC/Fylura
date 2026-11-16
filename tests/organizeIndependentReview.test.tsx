// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChromeProvider, type ChromeContextValue } from "../src/contexts/AppContexts";
import { SETTINGS_SECTION_EVENT } from "../src/components/spotlight/commandRegistry";
import { makeTranslator } from "../src/i18n";
import { useFileLibraryQueryStore, useFileLibraryResultStore, useFileLibrarySelectionStore } from "../src/store/useFileLibraryV2Store";
import { useOrganizationPlanStore } from "../src/store/useOrganizationPlanStore";
import type { AIProductFeatureReadiness, OrganizationPlan, OrganizationPlanGroupSummary, OrganizationPlanItem } from "../src/types/domain";
import { OrganizeSuggestionsView } from "../src/views/organize/OrganizeSuggestionsView";

const apiMocks = vi.hoisted(() => ({
  getAIFeatureReadiness: vi.fn(),
  listOrganizationPlans: vi.fn(),
  getOrganizationPlan: vi.fn(),
  createOrganizationPlan: vi.fn(),
  getOrganizationPlanDryRun: vi.fn(),
  executeOrganizationPlan: vi.fn(),
  queryOrganizationPlanGroupItems: vi.fn(),
  queryOrganizationPlanGroups: vi.fn(),
  updateOrganizationPlanGroupDecision: vi.fn(),
  updateOrganizationPlanDecisions: vi.fn(),
  cancelOrganizationPlan: vi.fn()
}));

vi.mock("../src/api/tauriApi", () => ({
  tauriApi: {
    getAIFeatureReadiness: apiMocks.getAIFeatureReadiness,
    listOrganizationPlans: apiMocks.listOrganizationPlans,
    getOrganizationPlan: apiMocks.getOrganizationPlan,
    createOrganizationPlan: apiMocks.createOrganizationPlan,
    getOrganizationPlanDryRun: apiMocks.getOrganizationPlanDryRun,
    executeOrganizationPlan: apiMocks.executeOrganizationPlan,
    queryOrganizationPlanGroupItems: apiMocks.queryOrganizationPlanGroupItems,
    queryOrganizationPlanGroups: apiMocks.queryOrganizationPlanGroups,
    updateOrganizationPlanGroupDecision: apiMocks.updateOrganizationPlanGroupDecision,
    updateOrganizationPlanDecisions: apiMocks.updateOrganizationPlanDecisions,
    cancelOrganizationPlan: apiMocks.cancelOrganizationPlan
  }
}));

const t = makeTranslator("zh");
const chrome = { t, setView: vi.fn(), language: "zh", view: "organize" } as unknown as ChromeContextValue;
const realOpenPlan = useOrganizationPlanStore.getState().openPlan;
let root: Root;
let container: HTMLDivElement;
const nativeGetBoundingClientRect = HTMLElement.prototype.getBoundingClientRect;
const nativeClientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
const nativeOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
const nativeOffsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetWidth");

function readyReadiness(): AIProductFeatureReadiness {
  const provider: AIProductFeatureReadiness["provider"] = {
    state: "ready",
    reason: "provider_configuration_ready",
    providerMode: "local",
    providerKind: "ollama",
    providerPreset: "ollama",
    model: "fixture-model",
    credentialRequired: false,
    credentialConfigured: false,
    settingsRevision: "fixture",
    bindingFingerprint: "fixture"
  };
  const disclosure = {
    providerPayloadIncludesFileName: false,
    providerPayloadIncludesParentPath: false,
    providerPayloadIncludesFullPath: false,
    providerPayloadIncludesFileContent: false,
    providerContentIsBounded: true
  };
  return {
    provider,
    managedScopes: [{
      state: "ready",
      reason: "managed_scope_ready",
      provider,
      managedScopeId: "managed-scope-fixture",
      scopeFingerprint: "scope-fixture",
      bindingFingerprint: "fixture",
      disclosure
    }],
    cleanup: {
      state: "ready",
      reason: "cleanup_ai_ready",
      provider,
      cleanupAiEnabled: true,
      localAiAllowed: true,
      cloudAiAllowed: false,
      bindingFingerprint: "fixture",
      disclosure
    }
  };
}

const plan: OrganizationPlan = {
  id: "plan-review",
  title: "Review plan",
  status: "ready",
  sourceKind: "explicit",
  sourceQueryFingerprint: null,
  sourceSnapshotRevision: 4,
  requestedCount: 1,
  materializedCount: 1,
  plannerVersion: 1,
  revision: 5,
  activeExecutionId: null,
  activeOperationBatchId: null,
  lastErrorCode: null,
  lastErrorDetail: null,
  createdAt: 1,
  updatedAt: 1,
  readyAt: 1,
  completedAt: null,
  effectiveSummary: { ready: 0, reviewed: 0, pendingReview: 1, blocked: 0 },
  summary: { undecided: 1, accepted: 0, kept: 0, edited: 0, needsAnalysis: 0, needsReview: 1, pendingReview: 1, reviewed: 0, ready: 0, blocked: 0, stale: 0, executing: 0, executed: 0, failed: 0, skipped: 0, remainingExecutable: 0 }
};

const reviewItem: OrganizationPlanItem = {
  id: "item-review",
  planId: plan.id,
  ordinal: 0,
  fileIdSnapshot: "file-review",
  sourcePathSnapshot: "C:/Inbox/report.txt",
  sourceNameSnapshot: "report.txt",
  sourceSizeSnapshot: 120,
  sourceMtimeSnapshot: 1,
  sourceIsDirSnapshot: false,
  proposalFingerprint: "proposal-review",
  semanticExplanation: null,
  proposalKind: "move",
  proposedTargetDirectory: "C:/Documents",
  proposedName: "report.txt",
  proposedTargetPath: "C:/Documents/report.txt",
  decision: "undecided",
  editedName: null,
  validity: "needs_review",
  reviewState: "needs_review",
  effectiveReadiness: "requires-decision",
  confidence: 0.7,
  riskLevel: "Normal",
  requiresConfirmation: true,
  blockingCode: null,
  blockingDetail: null,
  authoritativePreviewId: "preview-review",
  reviewReasons: ["low_confidence", "requires_confirmation"],
  availableActions: ["accept_suggestion", "edit_name", "view_preview", "keep", "defer"],
  operationLogId: null,
  executionId: null,
  revision: 2,
  createdAt: 1,
  updatedAt: 1
};

const reviewGroup: OrganizationPlanGroupSummary = {
  groupId: "group-review",
  planId: plan.id,
  label: "C:/Documents · move",
  targetDirectory: "C:/Documents",
  proposalKind: "move",
  readiness: "requires-decision",
  riskLevel: "Normal",
  itemCount: 1,
  totalBytes: 120,
  acceptedCount: 0,
  excludedCount: 0,
  staleCount: 0,
  conflictCount: 0,
  confidenceBand: "medium",
  reviewReasonCounts: [{ reason: "low_confidence", count: 1 }, { reason: "requires_confirmation", count: 1 }],
  availableActions: ["accept_suggestion", "edit_name", "keep", "defer"],
  groupActions: { canAcceptAll: false, canKeepAll: true, canClearAll: false },
  projectionFingerprint: "fp-review",
  sampleItems: [{ itemId: reviewItem.id, sourceName: reviewItem.sourceNameSnapshot, sourcePath: reviewItem.sourcePathSnapshot, proposedName: reviewItem.proposedName, decision: reviewItem.decision, validity: reviewItem.validity }],
  revision: plan.revision
};

const readyItem: OrganizationPlanItem = {
  ...reviewItem,
  id: "item-ready",
  validity: "ready",
  reviewState: "ready",
  effectiveReadiness: "ready",
  confidence: 0.95,
  requiresConfirmation: false,
  reviewReasons: [],
  availableActions: ["accept_suggestion", "edit_name", "view_preview", "keep"]
};

const readyGroup: OrganizationPlanGroupSummary = {
  ...reviewGroup,
  groupId: "group-ready",
  label: "C:/Documents · move · ready",
  readiness: "ready",
  confidenceBand: "high",
  reviewReasonCounts: [],
  availableActions: ["accept_suggestion", "edit_name", "keep"],
  groupActions: { canAcceptAll: true, canKeepAll: true, canClearAll: false },
  sampleItems: [{ ...reviewGroup.sampleItems[0], itemId: readyItem.id, decision: readyItem.decision, validity: readyItem.validity }]
};

const reviewedItem: OrganizationPlanItem = {
  ...reviewItem,
  decision: "accepted",
  reviewState: "reviewed",
  effectiveReadiness: "reviewed",
  revision: 3,
  availableActions: ["edit_name", "view_preview", "keep", "clear_decision"]
};

const reviewedGroup: OrganizationPlanGroupSummary = {
  ...reviewGroup,
  groupId: "group-reviewed",
  readiness: "reviewed",
  acceptedCount: 1,
  availableActions: ["edit_name", "keep", "clear_decision"],
  groupActions: { canAcceptAll: false, canKeepAll: true, canClearAll: true },
  sampleItems: [{ ...reviewGroup.sampleItems[0], itemId: reviewedItem.id, decision: "accepted", validity: "needs_review" }]
};

function updatedPlan(): OrganizationPlan {
  return { ...plan, revision: 6, effectiveSummary: { ready: 0, reviewed: 1, pendingReview: 0, blocked: 0 }, summary: { ...plan.summary, undecided: 0, accepted: 1, needsReview: 1, pendingReview: 0, reviewed: 1, remainingExecutable: 1 } };
}

function updatedItem(): OrganizationPlanItem {
  return { ...reviewItem, decision: "accepted", reviewState: "reviewed", revision: 3, availableActions: ["edit_name", "view_preview", "keep", "clear_decision"] };
}

function button(text: string): HTMLButtonElement {
  const found = [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.includes(text));
  if (!found) throw new Error(`Button not found: ${text}`);
  return found;
}

type ReactProps = { onClick?: (event?: unknown) => unknown; onChange?: (event: unknown) => unknown };

function reactProps(element: HTMLElement): ReactProps {
  const key = Object.keys(element).find((item) => item.startsWith("__reactProps$"));
  if (!key) throw new Error("React props not found");
  return (element as unknown as Record<string, ReactProps>)[key];
}

async function flush() {
  for (let index = 0; index < 5; index += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
}

describe("Organize independent review behavior", () => {
  let acceptedReview = false;

  beforeEach(() => {
    document.body.innerHTML = '<div id="test-root"></div>';
    window.sessionStorage.clear();
    container = document.getElementById("test-root") as HTMLDivElement;
    root = createRoot(container);
    apiMocks.listOrganizationPlans.mockReset().mockResolvedValue([]);
    apiMocks.getAIFeatureReadiness.mockReset().mockResolvedValue(readyReadiness());
    apiMocks.createOrganizationPlan.mockReset();
    HTMLElement.prototype.getBoundingClientRect = () => ({ width: 800, height: 600, top: 0, left: 0, right: 800, bottom: 600, x: 0, y: 0, toJSON() { return {}; } }) as DOMRect;
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, value: 600 });
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, value: 600 });
    Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, value: 800 });
    useFileLibraryQueryStore.setState({ fingerprint: "query", snapshotRevision: 1 });
    useFileLibraryResultStore.setState({ totalCount: 1 });
    useFileLibrarySelectionStore.setState({ selection: null, focusedId: "", anchorIndex: -1 });
    useOrganizationPlanStore.setState({
      plans: [plan],
      activePlan: plan,
      groups: [reviewGroup],
      groupProjectionFingerprint: null,
      groupHasMore: false,
      groupNextCursor: null,
      planListState: "loaded",
      planListError: null,
      planListRequestEpoch: 0,
      activePlanState: "loaded",
      openPlanError: null,
      openPlanErrorPlanId: null,
      createPlanError: null,
      isPlanListLoading: false,
      isLoading: false,
      isMutating: false,
      error: null,
      dryRun: null,
      executionResult: null,
      openPlan: vi.fn(async () => undefined),
      refreshPlan: vi.fn(async () => ({ applied: true as const, value: plan }))
    });
    acceptedReview = false;
    apiMocks.queryOrganizationPlanGroupItems.mockImplementation(async (request: { groupId?: string }) => ({
      planId: plan.id,
      groupId: request?.groupId ?? reviewGroup.groupId,
      planRevision: acceptedReview ? 6 : 5,
      projectionFingerprint: request?.groupId === reviewedGroup.groupId ? reviewedGroup.projectionFingerprint : request?.groupId === readyGroup.groupId ? readyGroup.projectionFingerprint : reviewGroup.projectionFingerprint,
      items: request?.groupId === reviewedGroup.groupId ? [reviewedItem] : request?.groupId === readyGroup.groupId ? [readyItem] : [reviewItem],
      nextCursor: null,
      hasMore: false
    }));
    apiMocks.queryOrganizationPlanGroups.mockImplementation(async () => ({ planId: plan.id, planRevision: acceptedReview ? 6 : 5, projectionFingerprint: acceptedReview ? "page-fp-reviewed" : "page-fp-review", groups: acceptedReview ? [reviewedGroup] : [reviewGroup], effectiveSummary: acceptedReview ? { ready: 0, reviewed: 1, pendingReview: 0, blocked: 0 } : plan.effectiveSummary, nextCursor: null, hasMore: false }));
    apiMocks.updateOrganizationPlanGroupDecision.mockResolvedValue({ plan, group: reviewGroup });
    apiMocks.updateOrganizationPlanDecisions.mockImplementation(async () => {
      acceptedReview = true;
      return updatedPlan();
    });
  });

  afterEach(() => {
    act(() => root.unmount());
    HTMLElement.prototype.getBoundingClientRect = nativeGetBoundingClientRect;
    if (nativeClientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", nativeClientHeight);
    else delete (HTMLElement.prototype as { clientHeight?: number }).clientHeight;
    if (nativeOffsetHeight) Object.defineProperty(HTMLElement.prototype, "offsetHeight", nativeOffsetHeight);
    else delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
    if (nativeOffsetWidth) Object.defineProperty(HTMLElement.prototype, "offsetWidth", nativeOffsetWidth);
    else delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
    document.body.innerHTML = "";
    window.sessionStorage.clear();
    vi.clearAllMocks();
  });

  it("disables the pending create button, preserves its title, and rejects a saved old handler", async () => {
    const created = { ...plan, id: "plan-created" };
    let resolveCreate: (value: OrganizationPlan) => void = () => undefined;
    const pendingCreate = new Promise<OrganizationPlan>((resolve) => { resolveCreate = resolve; });
    apiMocks.createOrganizationPlan.mockReturnValue(pendingCreate);
    useOrganizationPlanStore.setState({ plans: [], activePlan: null, groups: [], planListState: "loaded", planListError: null, createPlanError: null, isPlanListLoading: false, isLoading: false, isMutating: false, error: null });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    const input = container.querySelector<HTMLInputElement>('#organization-plan-title');
    expect(input).not.toBeNull();
    reactProps(input as HTMLInputElement).onChange?.({ target: { value: "Keep this title" }, currentTarget: { value: "Keep this title" } });
    await flush();
    const createButton = button(t("organizeCreatePlanAction"));
    const savedClick = reactProps(createButton).onClick;
    expect(savedClick).toBeTypeOf("function");

    await act(async () => { savedClick?.(); await Promise.resolve(); });
    expect(apiMocks.createOrganizationPlan).toHaveBeenCalledOnce();
    expect(createButton.disabled).toBe(true);
    expect(input?.value).toBe("Keep this title");

    await act(async () => { savedClick?.(); await Promise.resolve(); });
    expect(apiMocks.createOrganizationPlan).toHaveBeenCalledOnce();

    resolveCreate(created);
    await flush();
    expect(useOrganizationPlanStore.getState().isMutating).toBe(false);
  });

  it("offers a new plan when the listed plans are all terminal", async () => {
    const historicalPlan = { ...plan, id: "plan-completed", status: "completed", completedAt: 2 } as OrganizationPlan;
    const createdPlan = { ...plan, id: "plan-created", status: "ready" } as OrganizationPlan;
    apiMocks.listOrganizationPlans.mockResolvedValueOnce([historicalPlan]);
    apiMocks.createOrganizationPlan.mockResolvedValueOnce(createdPlan);
    useOrganizationPlanStore.setState({ plans: [historicalPlan], activePlan: null, groups: [], planListState: "loaded", activePlanState: "idle", isPlanListLoading: false, isLoading: false, isMutating: false });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();

    expect(container.querySelector("#organization-plan-title")).not.toBeNull();
    expect(container.textContent).toContain(t("organizeCreateAnotherPlanTitle"));
    expect(useOrganizationPlanStore.getState().openPlan).not.toHaveBeenCalled();

    await act(async () => button(t("organizeCreatePlanAction")).click());
    await flush();
    expect(apiMocks.createOrganizationPlan).toHaveBeenCalledOnce();
    expect(useOrganizationPlanStore.getState().activePlan?.id).toBe(createdPlan.id);
  });

  it("offers a new plan when the terminal active plan remains selected", async () => {
    const historicalPlan = { ...plan, id: "plan-completed", status: "completed", completedAt: 2 } as OrganizationPlan;
    const createdPlan = { ...plan, id: "plan-created", status: "ready" } as OrganizationPlan;
    apiMocks.listOrganizationPlans.mockResolvedValueOnce([historicalPlan]);
    apiMocks.createOrganizationPlan.mockResolvedValueOnce(createdPlan);
    useOrganizationPlanStore.setState({ plans: [historicalPlan], activePlan: historicalPlan, groups: [], planListState: "loaded", activePlanState: "loaded", isPlanListLoading: false, isLoading: false, isMutating: false });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();

    expect(container.querySelector("#organization-plan-title")).not.toBeNull();
    expect(container.textContent).toContain(t("organizeCreateAnotherPlanTitle"));
    await act(async () => button(t("organizeCreatePlanAction")).click());
    await flush();
    expect(apiMocks.createOrganizationPlan).toHaveBeenCalledOnce();
  });

  it("keeps a partially completed plan reviewable but disables cancellation", async () => {
    const partialPlan = { ...plan, status: "partially_completed" } as OrganizationPlan;
    apiMocks.listOrganizationPlans.mockResolvedValueOnce([partialPlan]);
    apiMocks.cancelOrganizationPlan.mockResolvedValueOnce({ ...partialPlan, status: "cancelled" });
    useOrganizationPlanStore.setState({ plans: [partialPlan], activePlan: partialPlan, groups: [], planListState: "loaded", activePlanState: "loaded", isPlanListLoading: false, isLoading: false, isMutating: false });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();

    expect(button(t("organizePlanCancel")).disabled).toBe(true);
    await act(async () => {
      await useOrganizationPlanStore.getState().cancelPlan();
    });
    expect(apiMocks.cancelOrganizationPlan).not.toHaveBeenCalled();
  });

  it("renders the workflow boundary and keeps blocked suggestions out of execution", async () => {
    const blockedPlan: OrganizationPlan = {
      ...plan,
      effectiveSummary: { ready: 0, reviewed: 0, pendingReview: 0, blocked: 1 },
      summary: { ...plan.summary, needsReview: 0, pendingReview: 0, blocked: 1, remainingExecutable: 0 }
    };
    const blockedGroup: OrganizationPlanGroupSummary = {
      ...reviewGroup,
      readiness: "blocked",
      availableActions: ["keep"],
      groupActions: { canAcceptAll: false, canKeepAll: true, canClearAll: false },
      reviewReasonCounts: [{ reason: "target_collision", count: 1 }]
    };
    apiMocks.listOrganizationPlans.mockResolvedValueOnce([blockedPlan]);
    useOrganizationPlanStore.setState({ plans: [blockedPlan], activePlan: blockedPlan, groups: [blockedGroup], activePlanState: "loaded", planListState: "loaded", isLoading: false, isMutating: false });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();

    expect(container.querySelectorAll("[data-workflow-steps]")).toHaveLength(1);
    expect(container.querySelector('[data-workflow-steps] [data-workflow-state="blocked"]')).not.toBeNull();
    expect(container.querySelector('[data-workflow-steps] [data-workflow-state="done"]')?.getAttribute("aria-label")).toContain(t("workflowStateDone"));
    expect(container.querySelector('[data-workflow-steps] [data-workflow-state="blocked"]')?.getAttribute("aria-label")).toContain(t("workflowStateBlocked"));
    expect(container.querySelectorAll("[data-workflow-marker]")).toHaveLength(4);
    expect(container.querySelector('[data-workflow-marker="blocked"] svg')?.getAttribute("aria-hidden")).toBe("true");
    expect(container.querySelector("[data-organize-execution-boundary]")).not.toBeNull();
    expect(button(t("organizeReviewExecution")).disabled).toBe(true);
    expect(apiMocks.getOrganizationPlanDryRun).not.toHaveBeenCalled();
  });

  it("keeps execution behind an authoritative Dry Run", async () => {
    const readyPlan: OrganizationPlan = {
      ...plan,
      effectiveSummary: { ready: 1, reviewed: 0, pendingReview: 0, blocked: 0 },
      summary: { ...plan.summary, undecided: 0, accepted: 1, needsReview: 0, pendingReview: 0, reviewed: 1, ready: 1, remainingExecutable: 1 }
    };
    const dryRun = {
      planId: readyPlan.id,
      planRevision: readyPlan.revision,
      selectedCount: 1,
      executableCount: 1,
      blockedCount: 0,
      staleCount: 0,
      totalBytes: 120,
      operationKinds: ["move"],
      items: [],
      executionBatchLimit: 100,
      dryRunFingerprint: "dry-run-ready"
    };
    apiMocks.listOrganizationPlans.mockResolvedValueOnce([readyPlan]);
    apiMocks.getOrganizationPlanDryRun.mockResolvedValueOnce(dryRun);
    useOrganizationPlanStore.setState({ plans: [readyPlan], activePlan: readyPlan, groups: [readyGroup], activePlanState: "loaded", planListState: "loaded", isLoading: false, isMutating: false, dryRun: null, executionResult: null });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button(t("organizeReviewExecution")).click());
    await flush();

    expect(apiMocks.getOrganizationPlanDryRun).toHaveBeenCalledOnce();
    expect(container.textContent).toContain(t("organizeDryRunTitle"));
    expect(container.querySelector('[data-workflow-steps] [data-workflow-state="current"]')).not.toBeNull();
    expect(container.querySelector('[data-workflow-steps] [data-workflow-state="current"]')?.getAttribute("aria-label")).toContain(t("workflowStateCurrent"));
    expect(container.querySelector('[data-workflow-steps] [data-workflow-state="pending"]')?.getAttribute("aria-label")).toContain(t("workflowStatePending"));
    expect(container.querySelectorAll("[data-workflow-marker]")).toHaveLength(4);
    expect(apiMocks.executeOrganizationPlan).not.toHaveBeenCalled();
  });

  it("keeps execution unavailable when the authoritative Dry Run fails", async () => {
    const readyPlan: OrganizationPlan = {
      ...plan,
      effectiveSummary: { ready: 1, reviewed: 0, pendingReview: 0, blocked: 0 },
      summary: { ...plan.summary, undecided: 0, accepted: 1, needsReview: 0, pendingReview: 0, reviewed: 1, ready: 1, remainingExecutable: 1 }
    };
    apiMocks.listOrganizationPlans.mockResolvedValueOnce([readyPlan]);
    apiMocks.getOrganizationPlanDryRun.mockRejectedValueOnce(new Error("dry_run_unavailable"));
    useOrganizationPlanStore.setState({ plans: [readyPlan], activePlan: readyPlan, groups: [readyGroup], activePlanState: "loaded", planListState: "loaded", isLoading: false, isMutating: false, dryRun: null, executionResult: null });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button(t("organizeReviewExecution")).click());
    await flush();

    expect(apiMocks.getOrganizationPlanDryRun).toHaveBeenCalledOnce();
    expect(container.textContent).toContain(t("organizePreviewUnavailableTitle"));
    expect(apiMocks.executeOrganizationPlan).not.toHaveBeenCalled();
  });

  it("owns Dry Run errors by plan and revision and clears them after a successful retry", async () => {
    const readySummary = { ready: 1, reviewed: 0, pendingReview: 0, blocked: 0 };
    const readyPlan = (id: string, revision: number): OrganizationPlan => ({
      ...plan,
      id,
      title: id,
      revision,
      effectiveSummary: readySummary,
      summary: { ...plan.summary, undecided: 0, accepted: 1, needsReview: 0, pendingReview: 0, reviewed: 1, ready: 1, remainingExecutable: 1 }
    });
    const planA = readyPlan("plan-a", 5);
    const planB = readyPlan("plan-b", 9);
    const planBRevision = readyPlan("plan-b", 10);
    const groupA = { ...readyGroup, planId: planA.id, groupId: "group-a", revision: planA.revision };
    const groupB = { ...readyGroup, planId: planB.id, groupId: "group-b", revision: planB.revision };
    const groupBRevision = { ...groupB, revision: planBRevision.revision };
    const dryRun = {
      planId: planBRevision.id,
      planRevision: planBRevision.revision,
      selectedCount: 1,
      executableCount: 1,
      blockedCount: 0,
      staleCount: 0,
      totalBytes: 120,
      operationKinds: ["move"],
      items: [],
      executionBatchLimit: 100,
      dryRunFingerprint: "dry-run-plan-b-revision-10"
    };
    apiMocks.listOrganizationPlans.mockResolvedValueOnce([planA, planB]);
    apiMocks.getOrganizationPlanDryRun
      .mockReset()
      .mockRejectedValueOnce(new Error("dry_run_plan_a"))
      .mockRejectedValueOnce(new Error("dry_run_plan_b"))
      .mockResolvedValueOnce(dryRun);
    useOrganizationPlanStore.setState({ plans: [planA, planB], activePlan: planA, groups: [groupA], activePlanState: "loaded", planListState: "loaded", isLoading: false, isMutating: false, dryRun: null, executionResult: null, error: null });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button(t("organizeReviewExecution")).click());
    await flush();
    expect(container.querySelector('[data-durable-task="failed"]')).not.toBeNull();

    useOrganizationPlanStore.setState({ activePlan: planB, groups: [groupB], dryRun: null, error: null, isMutating: false });
    await flush();
    expect(container.querySelector('[data-durable-task="failed"]')).toBeNull();

    await act(async () => button(t("organizeReviewExecution")).click());
    await flush();
    expect(container.querySelector('[data-durable-task="failed"]')).not.toBeNull();

    useOrganizationPlanStore.setState({ activePlan: planBRevision, groups: [groupBRevision], dryRun: null, error: null, isMutating: false });
    await flush();
    expect(container.querySelector('[data-durable-task="failed"]')).toBeNull();

    await act(async () => button(t("organizeReviewExecution")).click());
    await flush();
    expect(apiMocks.getOrganizationPlanDryRun).toHaveBeenCalledTimes(3);
    expect(container.textContent).toContain(t("organizeDryRunTitle"));
    expect(container.querySelector('[data-durable-task="failed"]')).toBeNull();
  });

  it("allows a create-plan retry after the first backend failure", async () => {
    const created = { ...plan, id: "plan-retried" };
    const backendError = "sqlite_error: C:\\Users\\name\\secret.db internal_code_42";
    apiMocks.createOrganizationPlan
      .mockRejectedValueOnce(new Error(backendError))
      .mockResolvedValueOnce(created);
    useOrganizationPlanStore.setState({ plans: [], activePlan: null, groups: [], planListState: "loaded", planListError: null, createPlanError: null, isPlanListLoading: false, isLoading: false, isMutating: false, error: null });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button(t("organizeCreatePlanAction")).click());
    await flush();
    expect(apiMocks.createOrganizationPlan).toHaveBeenCalledOnce();
    expect(useOrganizationPlanStore.getState().isMutating).toBe(false);
    expect(container.textContent).toContain(t("organizeCreatePlanFailedDesc"));
    expect(container.textContent).not.toContain(backendError);
    expect(container.textContent).not.toContain("secret.db");
    expect(container.textContent).not.toContain("internal_code_42");
    expect(useOrganizationPlanStore.getState().createPlanError).toContain(backendError);
    expect(button(t("organizeCreatePlanAction")).disabled).toBe(false);

    await act(async () => button(t("organizeCreatePlanAction")).click());
    await flush();
    expect(apiMocks.createOrganizationPlan).toHaveBeenCalledTimes(2);
  });

  it("keeps the create form hidden when Plan List loading fails and retries only the list", async () => {
    const backendError = "sqlite_error: C:\\Users\\name\\secret.db internal_code_42";
    apiMocks.listOrganizationPlans.mockRejectedValueOnce(new Error(backendError));
    useOrganizationPlanStore.setState({ plans: [], activePlan: null, groups: [], planListState: "idle", planListError: null, activePlanState: "idle", openPlanError: null, openPlanErrorPlanId: null, createPlanError: null, isPlanListLoading: false, isLoading: false, isMutating: false, error: null });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();

    expect(container.textContent).toContain(t("organizePlanListFailedDesc"));
    expect(container.textContent).not.toContain(backendError);
    expect(container.textContent).not.toContain("secret.db");
    expect(container.textContent).not.toContain("internal_code_42");
    expect(useOrganizationPlanStore.getState().planListError).toContain(backendError);
    expect(container.textContent).not.toContain(t("organizeCreatePlanAction"));
    expect(container.querySelector("#organization-plan-title")).toBeNull();
    expect(apiMocks.createOrganizationPlan).not.toHaveBeenCalled();
    const retry = button(t("organizePlanListRetry"));
    apiMocks.listOrganizationPlans.mockResolvedValueOnce([]);
    await act(async () => retry.click());
    await flush();

    expect(container.querySelector("#organization-plan-title")).not.toBeNull();
    expect(apiMocks.listOrganizationPlans).toHaveBeenCalledTimes(2);
  });

  it("opens the first plan after a successful Plan List retry instead of showing creation", async () => {
    apiMocks.listOrganizationPlans.mockRejectedValueOnce(new Error("plan_list_failed")).mockResolvedValueOnce([plan]);
    useOrganizationPlanStore.setState({ plans: [], activePlan: null, groups: [], planListState: "idle", planListError: null, activePlanState: "idle", openPlanError: null, openPlanErrorPlanId: null, createPlanError: null, isPlanListLoading: false, isLoading: false, isMutating: false, error: null });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button(t("organizePlanListRetry")).click());
    await flush();

    expect(container.querySelector("#organization-plan-title")).toBeNull();
    expect(apiMocks.createOrganizationPlan).not.toHaveBeenCalled();
    expect(useOrganizationPlanStore.getState().openPlan).toHaveBeenCalledWith(plan.id);
  });

  it("shows an existing Plan open failure with retry and lets the user choose another plan", async () => {
    const otherPlan = { ...plan, id: "plan-other", title: "Other plan", revision: 6 };
    const backendError = "sqlite_error: C:\\Users\\name\\secret.db internal_code_42";
    apiMocks.listOrganizationPlans.mockResolvedValue([plan, otherPlan]);
    apiMocks.getOrganizationPlan
      .mockRejectedValueOnce(new Error(backendError))
      .mockResolvedValueOnce(plan)
      .mockResolvedValueOnce(otherPlan);
    apiMocks.queryOrganizationPlanGroups.mockImplementation(async (request: { planId: string }) => ({
      planId: request.planId,
      planRevision: request.planId === otherPlan.id ? otherPlan.revision : plan.revision,
      projectionFingerprint: request.planId === otherPlan.id ? "page-fp-other" : "page-fp-plan",
      groups: request.planId === otherPlan.id ? [readyGroup] : [reviewGroup],
      effectiveSummary: { ready: request.planId === otherPlan.id ? 1 : 0, reviewed: 0, pendingReview: request.planId === otherPlan.id ? 0 : 1, blocked: 0 },
      nextCursor: null,
      hasMore: false
    }));
    useOrganizationPlanStore.setState({ plans: [plan, otherPlan], activePlan: null, groups: [], planListState: "loaded", planListError: null, activePlanState: "idle", openPlanError: null, openPlanErrorPlanId: null, isPlanListLoading: false, isLoading: false, isMutating: false, error: null, openPlan: realOpenPlan });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();

    expect(container.textContent).toContain(t("organizePlanOpenFailedTitle"));
    expect(container.textContent).toContain(t("organizePlanOpenFailedDesc"));
    expect(container.textContent).not.toContain(backendError);
    expect(container.textContent).not.toContain("secret.db");
    expect(container.textContent).not.toContain("internal_code_42");
    expect(useOrganizationPlanStore.getState().openPlanError).toContain(backendError);
    expect(container.textContent).toContain(t("organizePlanOpenRetry"));
    expect(container.textContent).not.toContain(t("organizePlanOpening"));
    const failedSelector = container.querySelector<HTMLSelectElement>("#organization-plan-open-selector");
    expect(failedSelector).not.toBeNull();

    await act(async () => button(t("organizePlanOpenRetry")).click());
    await flush();
    expect(useOrganizationPlanStore.getState().activePlan?.id).toBe(plan.id);
    expect(container.textContent).not.toContain(t("organizePlanOpenFailedTitle"));

    const selector = container.querySelector<HTMLSelectElement>("#organization-plan-selector");
    expect(selector).not.toBeNull();

    await act(async () => {
      selector!.value = otherPlan.id;
      selector!.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await flush();

    expect(useOrganizationPlanStore.getState().activePlan?.id).toBe(otherPlan.id);
    expect(container.textContent).not.toContain(t("organizePlanOpenFailedTitle"));
    expect(container.textContent).toContain(otherPlan.title);
  });

  it("clears Group A items before loading Group B and leaves the workspace empty on a Group B failure", async () => {
    const groupA: OrganizationPlanGroupSummary = {
      ...readyGroup,
      groupId: "group-a",
      label: "C:/GroupA · move",
      targetDirectory: "C:/GroupA",
      projectionFingerprint: "fp-a",
      sampleItems: [{ ...readyGroup.sampleItems[0], itemId: "item-a", sourceName: "group-a.txt", sourcePath: "C:/GroupA/group-a.txt" }]
    };
    const groupB: OrganizationPlanGroupSummary = {
      ...readyGroup,
      groupId: "group-b",
      label: "C:/GroupB · move",
      targetDirectory: "C:/GroupB",
      projectionFingerprint: "fp-b",
      sampleItems: [{ ...readyGroup.sampleItems[0], itemId: "item-b", sourceName: "group-b.txt", sourcePath: "C:/GroupB/group-b.txt" }]
    };
    const itemA = { ...readyItem, id: "item-a", sourceNameSnapshot: "group-a.txt", sourcePathSnapshot: "C:/GroupA/group-a.txt" };
    const itemB = { ...readyItem, id: "item-b", sourceNameSnapshot: "group-b.txt", sourcePathSnapshot: "C:/GroupB/group-b.txt" };
    let rejectGroupB: (reason: unknown) => void = () => undefined;
    const pendingGroupB = new Promise<never>((_, reject) => { rejectGroupB = reject; });
    useOrganizationPlanStore.setState({ groups: [groupA, groupB] });
    apiMocks.queryOrganizationPlanGroupItems.mockImplementation((request: { groupId?: string }) => request.groupId === groupA.groupId
      ? Promise.resolve({ planId: plan.id, groupId: groupA.groupId, planRevision: plan.revision, projectionFingerprint: groupA.projectionFingerprint, items: [itemA], nextCursor: null, hasMore: false })
      : pendingGroupB);

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => container.querySelector<HTMLElement>(`[data-organize-group-row="${groupA.groupId}"]`)?.click());
    await flush();
    expect(container.querySelector('[role="dialog"]')?.textContent).toContain(itemA.sourceNameSnapshot);

    await act(async () => container.querySelector<HTMLElement>(`[data-organize-group-row="${groupB.groupId}"]`)?.click());
    expect(container.querySelector('[role="dialog"]')?.textContent).not.toContain(itemA.sourceNameSnapshot);
    expect(container.textContent).toContain("C:/GroupB");

    rejectGroupB(new Error("group_b_items_failed"));
    await flush();
    expect(container.textContent).toContain(t("organizeLoadFailedDesc"));
    expect(container.querySelector('[role="dialog"]')?.textContent).not.toContain(itemA.sourceNameSnapshot);
  });

  it("keeps group acceptance out of requires-decision when the backend action intersection is unavailable and confirms a single ordinary item mutation", async () => {
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button("需要我决定").click());
    await flush();

    expect(container.textContent).toContain("置信度较低 (1)");
    expect(container.textContent).toContain("需要确认 (1)");
    expect([...container.querySelectorAll<HTMLButtonElement>("button")].some((item) => item.textContent?.includes("纳入整组"))).toBe(false);
    await act(async () => container.querySelector<HTMLElement>(`[data-organize-group-row="${reviewGroup.groupId}"]`)?.click());
    await flush();

    const accept = button("接受此建议");
    expect(accept.disabled).toBe(false);
    await act(async () => accept.click());
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("置信度较低");
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("C:/Documents/report.txt");
    expect(apiMocks.updateOrganizationPlanDecisions).not.toHaveBeenCalled();

    const confirm = [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find((item) => item.textContent?.includes("确认接受建议"));
    expect(confirm).toBeTruthy();
    await act(async () => confirm?.click());
    await flush();

    expect(apiMocks.updateOrganizationPlanDecisions).toHaveBeenCalledWith({
      planId: plan.id,
      expectedPlanRevision: plan.revision,
      mutations: [{ itemId: reviewItem.id, expectedItemRevision: reviewItem.revision, decision: "accepted", editedFilename: null }]
    });
    expect(apiMocks.updateOrganizationPlanDecisions.mock.calls[0]?.[0]?.safeBatch).not.toBe(true);
    expect(container.textContent).toContain("没有需要你决定的分组");
    expect(container.textContent).toContain("待你决定0");
    const dryRun = button("检查执行");
    expect(dryRun.disabled).toBe(false);
    expect(useOrganizationPlanStore.getState().groups[0]?.readiness).toBe("reviewed");
  });

  it("blocks new semantic analysis when provider readiness is unavailable but keeps current review and explanation usable", async () => {
    const provider = { ...readyReadiness().provider, state: "disabled" as const, reason: "provider_disabled" };
    apiMocks.getAIFeatureReadiness.mockResolvedValue({
      ...readyReadiness(),
      provider,
      managedScopes: [],
      cleanup: { ...readyReadiness().cleanup, state: "disabled", reason: "provider_disabled", provider, cleanupAiEnabled: false }
    });
    const needsAnalysisPlan = { ...plan, summary: { ...plan.summary, needsAnalysis: 1 } };
    const currentSemanticItem = {
      ...reviewItem,
      semanticExplanation: {
        assessmentFingerprint: "assessment-bound-to-current-proposal",
        reason: "Current Managed AI assessment explanation"
      }
    };
    apiMocks.queryOrganizationPlanGroupItems.mockResolvedValue({
      planId: plan.id,
      groupId: reviewGroup.groupId,
      planRevision: plan.revision,
      projectionFingerprint: reviewGroup.projectionFingerprint,
      items: [currentSemanticItem],
      nextCursor: null,
      hasMore: false
    });
    useOrganizationPlanStore.setState({ plans: [needsAnalysisPlan], activePlan: needsAnalysisPlan, groups: [reviewGroup] });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();

    expect(container.querySelector('[data-ai-provider-state="disabled"]')).not.toBeNull();
    expect(container.querySelector('[data-ai-plan-readiness="needs_analysis"]')).not.toBeNull();
    await act(async () => button(t("aiReadinessOpenSettings")).click());
    expect(chrome.setView).toHaveBeenCalledWith("settings");
    expect(window.sessionStorage.getItem(SETTINGS_SECTION_EVENT)).toBe("settings-ai-provider");

    const planActions = [...container.querySelectorAll("summary")].find((item) => item.textContent?.includes(t("organizePlanActions")));
    await act(async () => planActions?.click());
    expect(button(t("organizePlanAnalyze")).disabled).toBe(true);
    await act(async () => button("需要我决定").click());
    await flush();
    await act(async () => container.querySelector<HTMLElement>(`[data-organize-group-row="${reviewGroup.groupId}"]`)?.click());
    await flush();
    await act(async () => button("report.txt").click());
    await flush();
    const explanation = container.querySelector<HTMLElement>("[data-organize-semantic-explanation]");
    expect(explanation?.getAttribute("data-semantic-explanation-current")).toBe("true");
    expect(explanation?.getAttribute("data-semantic-explanation-source")).toBe("backend-authority");
    expect(explanation?.textContent).toContain("Current Managed AI assessment explanation");
    expect(button("接受此建议").disabled).toBe(false);
  });

  it("requires confirmation before accepting a requires-decision group", async () => {
    const reviewActionGroup: OrganizationPlanGroupSummary = {
      ...reviewGroup,
      groupActions: { canAcceptAll: true, canKeepAll: true, canClearAll: false }
    };
    useOrganizationPlanStore.setState({ groups: [reviewActionGroup] });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button("需要我决定").click());
    await flush();

    await act(async () => button("纳入整组").click());
    await flush();
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("确认接受整组建议");
    expect(apiMocks.updateOrganizationPlanGroupDecision).not.toHaveBeenCalled();

    await act(async () => button("确认接受整组").click());
    await flush();
    expect(apiMocks.updateOrganizationPlanGroupDecision).toHaveBeenCalledWith({
      planId: plan.id,
      groupId: reviewActionGroup.groupId,
      expectedPlanRevision: plan.revision,
      expectedProjectionFingerprint: reviewActionGroup.projectionFingerprint,
      expectedItemCount: reviewActionGroup.itemCount,
      decision: "accepted"
    });
  });

  it("keeps group acceptance available for a ready group and refreshes the plan after group acceptance", async () => {
    useOrganizationPlanStore.setState({ groups: [readyGroup] });
    apiMocks.queryOrganizationPlanGroupItems.mockResolvedValue({ planId: plan.id, groupId: readyGroup.groupId, planRevision: plan.revision, projectionFingerprint: readyGroup.projectionFingerprint, items: [readyItem], nextCursor: null, hasMore: false });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    const include = button("纳入整组");
    expect(include.disabled).toBe(false);
    await act(async () => include.click());
    await flush();

    expect(apiMocks.updateOrganizationPlanGroupDecision).toHaveBeenCalledWith({
      planId: plan.id,
      groupId: readyGroup.groupId,
      expectedPlanRevision: plan.revision,
      expectedProjectionFingerprint: readyGroup.projectionFingerprint,
      expectedItemCount: readyGroup.itemCount,
      decision: "accepted"
    });
    expect(useOrganizationPlanStore.getState().openPlan).toHaveBeenCalledWith(plan.id);
  });

  it("uses the backend group action intersection for mixed and keep groups", async () => {
    const mixedGroup: OrganizationPlanGroupSummary = {
      ...readyGroup,
      groupId: "group-mixed",
      groupActions: { canAcceptAll: false, canKeepAll: false, canClearAll: false }
    };
    useOrganizationPlanStore.setState({ groups: [mixedGroup] });
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    expect([...container.querySelectorAll<HTMLButtonElement>("button")].some((item) => item.textContent?.includes("纳入整组"))).toBe(false);
    expect([...container.querySelectorAll<HTMLButtonElement>("button")].some((item) => item.textContent?.includes("全部保留原位"))).toBe(false);
    act(() => root.unmount());
    root = createRoot(container);

    const keepGroup: OrganizationPlanGroupSummary = {
      ...readyGroup,
      groupId: "group-keep",
      proposalKind: "keep",
      groupActions: { canAcceptAll: false, canKeepAll: true, canClearAll: false }
    };
    const keepItem: OrganizationPlanItem = { ...readyItem, id: "item-keep", proposalKind: "keep", availableActions: ["keep"] };
    useOrganizationPlanStore.setState({ groups: [keepGroup] });
    apiMocks.queryOrganizationPlanGroupItems.mockResolvedValue({ planId: plan.id, groupId: keepGroup.groupId, planRevision: plan.revision, projectionFingerprint: keepGroup.projectionFingerprint, items: [keepItem], nextCursor: null, hasMore: false });
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    expect([...container.querySelectorAll<HTMLButtonElement>("button")].some((item) => item.textContent?.includes("纳入整组"))).toBe(false);
    expect(container.textContent).toContain("无需移动");
  });

  it("keeps an unavailable group action from refreshing or showing optimistic success", async () => {
    useOrganizationPlanStore.setState({ groups: [readyGroup] });
    apiMocks.queryOrganizationPlanGroupItems.mockResolvedValue({ planId: plan.id, groupId: readyGroup.groupId, planRevision: plan.revision, projectionFingerprint: readyGroup.projectionFingerprint, items: [readyItem], nextCursor: null, hasMore: false });
    apiMocks.updateOrganizationPlanGroupDecision.mockRejectedValueOnce(new Error("organization_group_action_not_available"));
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button("纳入整组").click());
    await flush();
    expect(container.textContent).toContain("该整组操作当前不可用，请查看分组详情。");
    expect(useOrganizationPlanStore.getState().openPlan).not.toHaveBeenCalled();
    expect(useOrganizationPlanStore.getState().refreshPlan).not.toHaveBeenCalled();
  });

  it("keeps reviewed groups in the plan tab and out of the decision tab", async () => {
    const reviewedPlan: OrganizationPlan = {
      ...plan,
      revision: 6,
      effectiveSummary: { ready: 0, reviewed: 1, pendingReview: 0, blocked: 0 },
      summary: { ...plan.summary, undecided: 0, accepted: 1, pendingReview: 0, reviewed: 1, remainingExecutable: 1 }
    };
    acceptedReview = true;
    useOrganizationPlanStore.setState({ plans: [reviewedPlan], activePlan: reviewedPlan, groups: [reviewedGroup] });

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    expect(container.querySelector(`[data-organize-group-row="${reviewedGroup.groupId}"]`)).toBeTruthy();
    expect(container.textContent).toContain("已复核");
    expect([...container.querySelectorAll<HTMLButtonElement>("button")].some((item) => item.textContent?.includes("纳入整组"))).toBe(false);
    await act(async () => button("需要我决定").click());
    await flush();
    expect(container.textContent).toContain("没有需要你决定的分组");
    expect(container.querySelector(`[data-organize-group-row="${reviewedGroup.groupId}"]`)).toBeNull();
  });

  it("renders an unavailable acceptance action when the item has no authoritative preview", async () => {
    const noPreview = { ...reviewItem, authoritativePreviewId: null, availableActions: ["keep", "defer"], reviewReasons: ["unknown_backend_reason"] };
    apiMocks.queryOrganizationPlanGroupItems.mockResolvedValue({ planId: plan.id, groupId: reviewGroup.groupId, planRevision: 5, projectionFingerprint: reviewGroup.projectionFingerprint, items: [noPreview], nextCursor: null, hasMore: false });
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button("需要我决定").click());
    await act(async () => container.querySelector<HTMLElement>(`[data-organize-group-row="${reviewGroup.groupId}"]`)?.click());
    await flush();
    const unavailable = button("当前不能接受");
    expect(unavailable.disabled).toBe(true);
    expect(unavailable.title).toContain("权威预览");
    expect(container.textContent).toContain("需要进一步复核");
  });

  it("does not offer acceptance for a collision item while keeping the edit action", async () => {
    const collisionItem: OrganizationPlanItem = {
      ...reviewItem,
      validity: "blocked",
      reviewState: "blocked",
      blockingCode: "target_collision",
      reviewReasons: ["target_collision"],
      availableActions: ["edit_name", "view_preview", "keep"]
    };
    const collisionGroup: OrganizationPlanGroupSummary = {
      ...reviewGroup,
      groupId: "group-collision",
      readiness: "blocked",
      reviewReasonCounts: [{ reason: "target_collision", count: 1 }],
      availableActions: ["edit_name", "keep"],
      groupActions: { canAcceptAll: false, canKeepAll: true, canClearAll: false },
      sampleItems: [{ ...reviewGroup.sampleItems[0], itemId: collisionItem.id, validity: "blocked" }]
    };
    useOrganizationPlanStore.setState({ groups: [collisionGroup] });
    apiMocks.queryOrganizationPlanGroups.mockResolvedValue({ planId: plan.id, planRevision: plan.revision, projectionFingerprint: "page-fp-collision", groups: [collisionGroup], effectiveSummary: { ready: 0, reviewed: 0, pendingReview: 0, blocked: 1 }, nextCursor: null, hasMore: false });
    apiMocks.queryOrganizationPlanGroupItems.mockResolvedValue({ planId: plan.id, groupId: collisionGroup.groupId, planRevision: plan.revision, projectionFingerprint: collisionGroup.projectionFingerprint, items: [collisionItem], nextCursor: null, hasMore: false });
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button("暂不可处理").click());
    await act(async () => container.querySelector<HTMLElement>(`[data-organize-group-row="${collisionGroup.groupId}"]`)?.click());
    await flush();
    const unavailable = button("当前不能接受");
    expect(unavailable.disabled).toBe(true);
    expect(button("调整文件名").disabled).toBe(false);
    expect(apiMocks.updateOrganizationPlanDecisions).not.toHaveBeenCalled();
  });

  it("shows a localized backend eligibility error without optimistic acceptance", async () => {
    apiMocks.updateOrganizationPlanDecisions.mockRejectedValueOnce(new Error("organization_item_accept_not_available"));
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button("需要我决定").click());
    await act(async () => container.querySelector<HTMLElement>(`[data-organize-group-row="${reviewGroup.groupId}"]`)?.click());
    await flush();
    await act(async () => button("接受此建议").click());
    const confirm = [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find((item) => item.textContent?.includes("确认接受建议"));
    await act(async () => confirm?.click());
    await flush();
    expect(apiMocks.updateOrganizationPlanDecisions).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("建议已不可用，状态没有被本地接受");
    expect(container.querySelector(`[data-organize-group-row="${reviewGroup.groupId}"]`)).toBeTruthy();
  });

  it("shows a localized group-change error with an explicit refresh and no retry", async () => {
    useOrganizationPlanStore.setState({ groups: [readyGroup] });
    apiMocks.queryOrganizationPlanGroups.mockResolvedValue({ planId: plan.id, planRevision: plan.revision, projectionFingerprint: "page-fp-ready", groups: [readyGroup], effectiveSummary: { ready: 1, reviewed: 0, pendingReview: 0, blocked: 0 }, nextCursor: null, hasMore: false });
    apiMocks.queryOrganizationPlanGroupItems.mockResolvedValue({ planId: plan.id, groupId: readyGroup.groupId, planRevision: plan.revision, projectionFingerprint: readyGroup.projectionFingerprint, items: [readyItem], nextCursor: null, hasMore: false });
    apiMocks.updateOrganizationPlanGroupDecision.mockRejectedValueOnce(new Error("organization_group_changed"));

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button("纳入整组").click());
    await flush();

    expect(apiMocks.updateOrganizationPlanGroupDecision).toHaveBeenCalledOnce();
    expect(useOrganizationPlanStore.getState().openPlan).not.toHaveBeenCalled();
    expect(container.textContent).toContain("此分组中的文件或建议已经变化，请刷新后重新确认。");
    const refresh = button("刷新当前事实");
    expect(refresh).toBeTruthy();
    await act(async () => refresh.click());
    expect(useOrganizationPlanStore.getState().refreshPlan).toHaveBeenCalledOnce();
    expect(apiMocks.updateOrganizationPlanGroupDecision).toHaveBeenCalledOnce();
  });

  it("does not continue a mounted Plan A item mutation after the user switches to Plan B", async () => {
    const planB: OrganizationPlan = { ...plan, id: "plan-b", title: "Plan B", revision: 9 };
    const groupB: OrganizationPlanGroupSummary = { ...readyGroup, planId: planB.id, groupId: "group-b", revision: planB.revision };
    let resolveMutation: (value: OrganizationPlan) => void = () => undefined;
    apiMocks.updateOrganizationPlanDecisions.mockImplementation(() => new Promise<OrganizationPlan>((resolve) => { resolveMutation = resolve; }));

    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(OrganizeSuggestionsView) })));
    await flush();
    await act(async () => button("需要我决定").click());
    await act(async () => container.querySelector<HTMLElement>(`[data-organize-group-row="${reviewGroup.groupId}"]`)?.click());
    await flush();
    await act(async () => button("接受此建议").click());
    await act(async () => button("确认接受建议").click());
    await flush();

    useOrganizationPlanStore.setState({ activePlan: planB, plans: [plan, planB], groups: [groupB], groupNextCursor: null, groupHasMore: false, isMutating: false });
    resolveMutation(updatedPlan());
    await flush();

    expect(apiMocks.queryOrganizationPlanGroups).not.toHaveBeenCalledWith({ planId: plan.id, pageSize: 100, cursor: null });
    expect(useOrganizationPlanStore.getState().activePlan?.id).toBe(planB.id);
    expect(container.textContent).not.toContain("建议或安全预览加载失败，请重试。");
  });
});
