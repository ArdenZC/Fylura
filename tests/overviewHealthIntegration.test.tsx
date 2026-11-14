// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChromeProvider, type ChromeContextValue } from "../src/contexts/AppContexts";
import { makeTranslator } from "../src/i18n";
import { useBackgroundIndexerStore } from "../src/store/useBackgroundIndexerStore";
import { useFileLibraryStore } from "../src/store/useFileLibraryStore";
import { useOperationQueueStore } from "../src/store/useOperationQueueStore";
import { useOrganizationPlanStore } from "../src/store/useOrganizationPlanStore";
import { useScanManagerStore } from "../src/store/useScanManagerStore";
import type { AnalysisRun, ContentScopePolicy, DashboardStats, OrganizationPlan } from "../src/types/domain";
import { ScannerView } from "../src/views/scanner/ScannerView";

const apiMocks = vi.hoisted(() => ({
  getGlobalIndexStatus: vi.fn(),
  listScanRoots: vi.fn(),
  listManagedScopes: vi.fn(),
  getActiveAnalysisRun: vi.fn(),
  listAnalysisRuns: vi.fn(),
  listContentRuns: vi.fn(),
  getContentScopePolicy: vi.fn(),
  listOrganizationPlans: vi.fn(),
  onWatcherReconciliationStatus: vi.fn(),
  onManagedScanEvent: vi.fn(),
  onAnalysisRunUpdated: vi.fn(),
  onAnalysisFindingsPublished: vi.fn()
}));

const eventMocks = vi.hoisted(() => ({
  watcher: null as ((event: any) => void) | null,
  managedScan: null as ((event: any) => void) | null,
  analysisRun: null as ((event: any) => void) | null,
  analysisFindings: null as ((event: any) => void) | null,
  unregistered: [] as string[]
}));

vi.mock("../src/api/tauriApi", () => ({ tauriApi: apiMocks }));

const t = makeTranslator("zh");
const en = makeTranslator("en");
const chrome = {
  t,
  language: "zh",
  setView: vi.fn(),
  view: "scanner"
} as unknown as ChromeContextValue;
const englishChrome = {
  ...chrome,
  t: en,
  language: "en"
} as unknown as ChromeContextValue;

const stats = (overrides: Partial<DashboardStats> = {}): DashboardStats => ({
  totalFiles: 20,
  totalSize: 2048,
  diskTotalSize: 0,
  diskFreeSize: 0,
  diskUsageRatio: 0,
  duplicateFiles: 0,
  largeFiles: 0,
  sensitiveFiles: 0,
  needsConfirmation: 0,
  byType: {},
  byLifecycle: {},
  lastScannedAt: "2026-08-02T00:00:00.000Z",
  ...overrides
});

const readyIndex = {
  platform: "browser",
  enabled: true,
  status: "ready",
  processedEntries: 20,
  collectionComplete: true,
  totalEntries: 20,
  indexedVolumes: 1,
  readyVolumes: 1,
  pendingVolumes: 0,
  lastSyncAt: 1,
  lastError: null
};

const reviewPlan = {
  id: "plan-overview",
  title: "Overview plan",
  status: "ready",
  summary: { pendingReview: 4 },
  effectiveSummary: null,
  updatedAt: 1
} as unknown as OrganizationPlan;

function contentPolicy(rootId: string, enabled: boolean): ContentScopePolicy {
  return {
    rootId,
    rootRevision: 1,
    enabled,
    extractorFamilies: ["plain_text"],
    maxBytes: 1024,
    maxChars: 1024,
    maxPages: 10,
    maxRows: 10,
    rawRetentionMode: "none",
    rawRetentionChars: 0,
    localAllowed: true,
    cloudAllowed: false,
    policyRevision: enabled ? 1 : 0,
    updatedAt: 1
  };
}

function cleanupRun(reviewCount: number, exactReclaimableBytes: number, potentialReclaimableBytes = exactReclaimableBytes, safeCount = 0, cautionCount = 0): AnalysisRun {
  const findingsCount = safeCount + reviewCount + cautionCount;
  return {
    id: "analysis-overview",
    requestKey: "analysis-overview-request",
    requestAttempt: 1,
    scope: { kind: "approved_cleanup_paths", paths: ["C:/Cleanup"] },
    scopeHash: "scope-overview",
    sourceSnapshot: {},
    sourceSnapshotHash: "snapshot-overview",
    detectorSet: ["cleanup_heuristics_v1:v1"],
    detectorSetHash: "detectors-overview",
    status: "completed",
    phase: "completed",
    revision: 1,
    cancelRequested: false,
    rerunRequired: false,
    detectorsTotal: 1,
    detectorsCompleted: 1,
    detectorsFailed: 0,
    findingsStaged: findingsCount,
    findingsPublished: findingsCount,
    safeCount,
    reviewCount,
    cautionCount,
    exactReclaimableBytes,
    potentialReclaimableBytes,
    warningCount: 0,
    errorCount: 0,
    startedAt: 1,
    finishedAt: 2,
    lastCheckpointAt: 2,
    createdAt: 1,
    updatedAt: 2,
    errorCode: null,
    errorMessage: null
  };
}

function configureHealth(overrides: {
  index?: Record<string, unknown>;
  roots?: Record<string, unknown>[];
  analysisRuns?: AnalysisRun[];
  activeAnalysisRun?: AnalysisRun | null;
  contentRuns?: Record<string, unknown>[];
  plans?: OrganizationPlan[];
  managedScopes?: Record<string, unknown>[];
  contentPolicies?: Record<string, ContentScopePolicy>;
} = {}) {
  apiMocks.listOrganizationPlans.mockResolvedValue(overrides.plans ?? []);
  apiMocks.getGlobalIndexStatus.mockResolvedValue({ ...readyIndex, ...overrides.index });
  apiMocks.listScanRoots.mockResolvedValue(overrides.roots ?? []);
  apiMocks.listManagedScopes.mockResolvedValue(overrides.managedScopes ?? []);
  apiMocks.getActiveAnalysisRun.mockResolvedValue(overrides.activeAnalysisRun ?? null);
  apiMocks.listAnalysisRuns.mockResolvedValue(overrides.analysisRuns ?? []);
  apiMocks.listContentRuns.mockResolvedValue(overrides.contentRuns ?? []);
  apiMocks.getContentScopePolicy.mockImplementation(async (rootId: string) => overrides.contentPolicies?.[rootId] ?? contentPolicy(rootId, false));
}

function resetStores() {
  useFileLibraryStore.setState({ scope: { kind: "all" }, stats: stats(), isClassifyingWithAI: false, aiClassificationProgress: null });
  useScanManagerStore.setState({
    selectedFolders: [],
    isScanning: false,
    isCancelingScan: false,
    scanState: { status: "idle", progress: null, entries: [], error: null }
  });
  useBackgroundIndexerStore.setState({ pendingRoots: [], currentRoot: null, isBackgroundIndexing: false, failedRoots: [] });
  useOperationQueueStore.setState({ operationLogs: [], operationProgress: null, activeOperationKind: null, listenersRegistered: false, registrationPromise: null });
  useOrganizationPlanStore.setState({ activePlan: null, plans: [] });
}

function configureEventListeners() {
  eventMocks.watcher = null;
  eventMocks.managedScan = null;
  eventMocks.analysisRun = null;
  eventMocks.analysisFindings = null;
  eventMocks.unregistered.length = 0;
  apiMocks.onWatcherReconciliationStatus.mockImplementation(async (handler: (event: any) => void) => {
    eventMocks.watcher = handler;
    return () => eventMocks.unregistered.push("watcher");
  });
  apiMocks.onManagedScanEvent.mockImplementation(async (handler: (event: any) => void) => {
    eventMocks.managedScan = handler;
    return () => eventMocks.unregistered.push("scan");
  });
  apiMocks.onAnalysisRunUpdated.mockImplementation(async (handler: (event: any) => void) => {
    eventMocks.analysisRun = handler;
    return () => eventMocks.unregistered.push("analysis");
  });
  apiMocks.onAnalysisFindingsPublished.mockImplementation(async (handler: (event: any) => void) => {
    eventMocks.analysisFindings = handler;
    return () => eventMocks.unregistered.push("findings");
  });
}

let root: Root;
let container: HTMLDivElement;
let visibilityStateDescriptor: PropertyDescriptor | undefined;

async function flush(useFakeTimers = false) {
  for (let index = 0; index < 4; index += 1) {
    await act(async () => {
      if (useFakeTimers) await Promise.resolve();
      else await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
}

async function renderOverview(context: ChromeContextValue = chrome, useFakeTimers = false) {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root.render(createElement(ChromeProvider, { value: context, children: createElement(ScannerView) }));
  });
  await flush(useFakeTimers);
}

function priorityTitle() {
  return container.querySelector("#overview-priority-title")?.textContent ?? "";
}

function coverageText() {
  // The section head is its own row inside the section; read the whole section
  // so the label, value and hint are all included.
  return container.querySelector("#overview-system-coverage-title")?.closest("section")?.textContent ?? "";
}

function reclaimableMetric(labelText = t("overviewMetricReclaimable")) {
  const strip = container.querySelector<HTMLElement>('[data-overview-status-strip="true"]');
  const label = Array.from(strip?.querySelectorAll<HTMLElement>("span") ?? [])
    .find((node) => node.textContent === labelText);
  const metric = label?.parentElement;
  const hint = metric
    ? Array.from(metric.querySelectorAll<HTMLElement>("span")).find((node) => node !== label)?.textContent ?? ""
    : "";
  return {
    value: metric?.querySelector("strong")?.textContent ?? "",
    hint,
    text: metric?.textContent ?? ""
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => { resolve = nextResolve; });
  return { promise, resolve };
}

describe("Overview durable health integration", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStores();
    configureEventListeners();
    visibilityStateDescriptor = Object.getOwnPropertyDescriptor(document, "visibilityState");
  });

  afterEach(() => {
    act(() => root?.unmount());
    container?.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
    if (visibilityStateDescriptor) {
      Object.defineProperty(document, "visibilityState", visibilityStateDescriptor);
    } else {
      Reflect.deleteProperty(document, "visibilityState");
    }
  });

  it("shows search settings for Global Index no_source", async () => {
    configureHealth({ index: { status: "unavailable", enabled: false, collectionComplete: false, totalEntries: 0, indexedVolumes: 0 } });
    await renderOverview();
    expect(priorityTitle()).toBe("还没有可搜索的位置");
    expect(container.querySelector('[data-overview-primary="true"]')?.textContent).toContain("检查搜索来源");
  });

  it("refreshes on activation and authoritative changes, then releases the visible-only timer and listeners", async () => {
    configureHealth({ roots: [{ id: "root-a", enabled: true }] });
    const intervalSpy = vi.spyOn(window, "setInterval");
    const clearIntervalSpy = vi.spyOn(window, "clearInterval");
    await renderOverview();

    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(1);
    expect(intervalSpy).toHaveBeenCalledTimes(1);
    expect(intervalSpy).toHaveBeenCalledWith(expect.any(Function), 60_000);
    expect(apiMocks.onWatcherReconciliationStatus).toHaveBeenCalledTimes(1);
    expect(apiMocks.onManagedScanEvent).toHaveBeenCalledTimes(1);
    expect(apiMocks.onAnalysisRunUpdated).toHaveBeenCalledTimes(1);
    expect(apiMocks.onAnalysisFindingsPublished).toHaveBeenCalledTimes(1);

    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
      eventMocks.watcher?.({
        scanRootId: "root-a",
        pending: true,
        needsReconciliation: true,
        healthStatus: "reconciliation_required",
        activeRunId: null,
        lastErrorCode: null,
        lastErrorMessage: null
      });
    });
    await flush();
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(1);

    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    await flush();
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(2);
    expect(intervalSpy).toHaveBeenCalledTimes(2);

    await act(async () => eventMocks.managedScan?.({ status: "completed" }));
    await flush();
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(3);

    act(() => root.unmount());
    await flush();
    expect(eventMocks.unregistered.sort()).toEqual(["analysis", "findings", "scan", "watcher"]);
    expect(clearIntervalSpy).toHaveBeenCalledTimes(2);
    await act(async () => eventMocks.analysisRun?.({ status: "completed" }));
    await flush();
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(3);
    intervalSpy.mockRestore();
  });

  it("runs its 60-second safety refresh only while visible and clears it on unmount", async () => {
    vi.useFakeTimers();
    configureHealth({ roots: [{ id: "root-a", enabled: true }] });
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    const intervalSpy = vi.spyOn(window, "setInterval");
    const clearIntervalSpy = vi.spyOn(window, "clearInterval");

    await renderOverview(chrome, true);
    expect(apiMocks.getGlobalIndexStatus).not.toHaveBeenCalled();
    expect(intervalSpy).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    await flush(true);
    expect(apiMocks.getGlobalIndexStatus).not.toHaveBeenCalled();

    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    await flush(true);
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(1);
    expect(intervalSpy).toHaveBeenCalledTimes(1);
    expect(intervalSpy).toHaveBeenLastCalledWith(expect.any(Function), 60_000);
    const firstTimer = intervalSpy.mock.results[0]?.value as number;

    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    await flush(true);
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(2);

    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    await flush(true);
    expect(clearIntervalSpy).toHaveBeenCalledWith(firstTimer);
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    await flush(true);
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(2);

    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    await flush(true);
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(3);
    expect(intervalSpy).toHaveBeenCalledTimes(2);
    const secondTimer = intervalSpy.mock.results[1]?.value as number;

    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    await flush(true);
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(4);

    act(() => root.unmount());
    await flush(true);
    expect(clearIntervalSpy).toHaveBeenCalledWith(secondTimer);
    expect(eventMocks.unregistered.sort()).toEqual(["analysis", "findings", "scan", "watcher"]);
    await act(async () => eventMocks.analysisRun?.({ status: "completed" }));
    await flush(true);
    expect(apiMocks.getGlobalIndexStatus).toHaveBeenCalledTimes(4);
  });

  it("shows the synchronization entry for watcher reconciliation", async () => {
    configureHealth({ roots: [{ enabled: true, needsReconciliation: true, watcherRevision: 3, watcherAppliedRevision: 2, healthStatus: "reconciliation_required" }] });
    await renderOverview();
    expect(priorityTitle()).toBe("托管位置需要重新同步");
  });

  it("keeps distinct watcher health states and routes every action to file-source settings", async () => {
    configureHealth({ roots: [{ enabled: true, needsReconciliation: true, watcherRevision: 3, watcherAppliedRevision: 2, healthStatus: "permission_required", lastErrorCode: "watcher_reconciliation_retry_exhausted" }] });
    await renderOverview();
    expect(priorityTitle()).toBe("托管位置需要权限");
    expect(coverageText()).toContain("1 个托管位置");
    expect(container.querySelector('[data-overview-primary="true"]')?.textContent).toContain("检查权限");
    await act(async () => container.querySelector<HTMLButtonElement>('[data-overview-primary="true"]')?.click());
    expect(chrome.setView).toHaveBeenCalledWith("settings");

    act(() => root.unmount());
    container.remove();
    resetStores();
    configureHealth({ roots: [{ enabled: true, needsReconciliation: true, watcherRevision: 3, watcherAppliedRevision: 2, healthStatus: "reconciliation_required", lastErrorCode: "watcher_reconciliation_retry_exhausted" }] });
    await renderOverview();
    expect(priorityTitle()).toBe("托管位置重试已停止");

    act(() => root.unmount());
    container.remove();
    resetStores();
    configureHealth({ roots: [{ enabled: true, healthStatus: "degraded", needsReconciliation: true, watcherRevision: 3, watcherAppliedRevision: 2, activeRunId: "scan-1" }] });
    await renderOverview();
    expect(priorityTitle()).toBe("托管位置覆盖不完整");
  });

  it("uses Content Scope Policy instead of Managed AI permissions for coverage", async () => {
    configureHealth({
      roots: [{ id: "root-a", enabled: true }],
      managedScopes: [{ enabled: true, allowLocalAi: true, allowCloudAi: true }],
      contentPolicies: { "root-a": contentPolicy("root-a", false) }
    });
    await renderOverview();
    expect(coverageText()).toContain("0 / 1");
  });

  it("counts a consented content root even when Managed AI is disabled", async () => {
    configureHealth({
      roots: [{ id: "root-a", enabled: true }],
      managedScopes: [{ enabled: true, allowLocalAi: false, allowCloudAi: false }],
      contentPolicies: { "root-a": contentPolicy("root-a", true) }
    });
    await renderOverview();
    expect(coverageText()).toContain("1 / 1");
  });

  it("counts enabled Content Scope Policy only for enabled durable roots", async () => {
    configureHealth({
      roots: [
        { id: "root-a", enabled: true },
        { id: "root-b", enabled: true },
        { id: "root-c", enabled: false }
      ],
      managedScopes: [{ enabled: true, allowLocalAi: false, allowCloudAi: false }],
      contentPolicies: {
        "root-a": contentPolicy("root-a", true),
        "root-b": contentPolicy("root-b", false),
        "root-c": contentPolicy("root-c", true)
      }
    });
    await renderOverview();
    expect(coverageText()).toContain("1 / 3");
    expect(apiMocks.getContentScopePolicy).toHaveBeenCalledTimes(2);
    expect(apiMocks.getContentScopePolicy).not.toHaveBeenCalledWith("root-c");
  });

  it("shows zero of zero when the durable scan-root universe is empty", async () => {
    configureHealth({ roots: [] });
    await renderOverview();
    expect(coverageText()).toContain("0 / 0");
    expect(apiMocks.getContentScopePolicy).not.toHaveBeenCalled();
  });

  it("shows unknown content coverage when the policy authority cannot be read", async () => {
    configureHealth({ roots: [{ id: "root-a", enabled: true }] });
    apiMocks.getContentScopePolicy.mockRejectedValue(new Error("policy unavailable"));
    await renderOverview();
    expect(coverageText()).toContain("状态读取中");
    expect(coverageText()).not.toContain("0 / 1");
  });

  it("keeps content coverage unknown when the durable scan-root authority fails", async () => {
    configureHealth({ roots: [{ id: "root-a", enabled: true }] });
    apiMocks.listScanRoots.mockRejectedValue(new Error("roots unavailable"));
    await renderOverview();
    expect(coverageText()).toContain("状态读取中");
  });

  it("does not let an older policy refresh overwrite the latest coverage", async () => {
    const firstPolicy = deferred<ContentScopePolicy>();
    const secondPolicy = deferred<ContentScopePolicy>();
    configureHealth({ roots: [{ id: "root-a", enabled: true }] });
    apiMocks.getContentScopePolicy
      .mockImplementationOnce(() => firstPolicy.promise)
      .mockImplementationOnce(() => secondPolicy.promise);

    await act(async () => {
      container = document.createElement("div");
      document.body.appendChild(container);
      root = createRoot(container);
      root.render(createElement(ChromeProvider, { value: chrome, children: createElement(ScannerView) }));
    });
    await act(async () => { await Promise.resolve(); });
    expect(apiMocks.getContentScopePolicy).toHaveBeenCalledTimes(1);

    await act(async () => eventMocks.watcher?.({
      scanRootId: "root-a",
      pending: true,
      needsReconciliation: true,
      healthStatus: "reconciliation_required",
      activeRunId: null,
      lastErrorCode: null,
      lastErrorMessage: null
    }));
    await flush();
    expect(apiMocks.getContentScopePolicy).toHaveBeenCalledTimes(2);

    secondPolicy.resolve(contentPolicy("root-a", true));
    await act(async () => { await Promise.resolve(); });
    await act(async () => { await Promise.resolve(); });
    expect(document.body.textContent).toContain("1 / 1");

    firstPolicy.resolve(contentPolicy("root-a", false));
    await act(async () => { await Promise.resolve(); });
    await act(async () => { await Promise.resolve(); });
    expect(document.body.textContent).toContain("1 / 1");
  });

  it("prioritizes operation attention over ordinary scan work", async () => {
    configureHealth();
    useOperationQueueStore.setState({ operationLogs: [{ status: "manual_review", restore_status: "none" }] as never[], operationProgress: null, activeOperationKind: null });
    expect(useOperationQueueStore.getState().operationProgress).toBeNull();
    await renderOverview();
    expect(useOperationQueueStore.getState().operationProgress).toBeNull();
    expect(priorityTitle()).toBe("有操作需要复核");
  });

  it("shows an active operation as running when no attention items exist", async () => {
    configureHealth();
    useOperationQueueStore.setState({ operationProgress: { kind: "execute", processed: 1, total: 2, currentPath: "C:/file.txt", batchId: "batch-1" } as never });
    await renderOverview();
    expect(priorityTitle()).toBe("文件操作正在进行");
  });

  it("keeps failed attention ahead of a simultaneously active operation", async () => {
    configureHealth();
    useOperationQueueStore.setState({
      operationProgress: { kind: "execute", processed: 1, total: 2, currentPath: "C:/file.txt", batchId: "batch-1" } as never,
      operationLogs: [{ operation_type: "move", status: "failed", restore_status: "none" }] as never[]
    });
    await renderOverview();
    expect(priorityTitle()).toBe("有操作需要复核");
  });

  it("uses persisted plan summary when effectiveSummary is deferred", async () => {
    configureHealth();
    useFileLibraryStore.setState({ stats: stats({ needsConfirmation: 99 }) });
    useOrganizationPlanStore.setState({ activePlan: reviewPlan, plans: [reviewPlan] });
    await renderOverview();
    expect(priorityTitle()).toBe("有 4 项需要你确认");
  });

  it("prefers a reviewable durable plan over a historical active plan", async () => {
    const historicalPlan = { ...reviewPlan, id: "plan-historical", status: "completed", summary: { pendingReview: 0 }, updatedAt: 2 } as OrganizationPlan;
    const reviewablePlan = { ...reviewPlan, id: "plan-reviewable", status: "ready", summary: { pendingReview: 3 }, updatedAt: 1 } as OrganizationPlan;
    configureHealth({ plans: [historicalPlan, reviewablePlan] });
    useOrganizationPlanStore.setState({ activePlan: historicalPlan, plans: [historicalPlan, reviewablePlan] });
    await renderOverview();
    expect(priorityTitle()).toBe("有 3 项需要你确认");
  });

  it("shows review work from an older durable plan when the newer plan is authoritative zero", async () => {
    const newerZero = { ...reviewPlan, id: "plan-newer-zero", status: "ready", summary: { pendingReview: 0 }, effectiveSummary: { ready: 0, reviewed: 0, pendingReview: 0, blocked: 0 }, updatedAt: 2 } as OrganizationPlan;
    const olderPending = { ...reviewPlan, id: "plan-older-pending", status: "ready", summary: { pendingReview: 5 }, effectiveSummary: null, updatedAt: 1 } as OrganizationPlan;
    configureHealth({ plans: [newerZero, olderPending] });
    await renderOverview();
    expect(priorityTitle()).toBe("有 5 项需要你确认");
  });

  it("uses the durable cleanup run from the backend health snapshot", async () => {
    const run = cleanupRun(3, 4096);
    configureHealth({ analysisRuns: [run] });
    await renderOverview();
    expect(priorityTitle()).toBe("有 3 项清理候选");
  });

  it("renders exact reclaimable bytes without an estimated hint", async () => {
    configureHealth({ analysisRuns: [cleanupRun(2, 4096, 8192)] });
    await renderOverview();

    expect(reclaimableMetric()).toMatchObject({ value: "4.0 KB", hint: "" });
    expect(reclaimableMetric().text).not.toContain("估算");
  });

  it("renders potential reclaimable bytes with visible Chinese estimate semantics", async () => {
    configureHealth({ analysisRuns: [cleanupRun(2, 0, 8192)] });
    await renderOverview();

    expect(reclaimableMetric()).toMatchObject({ value: "~8.0 KB", hint: "释放空间是估算值" });
    expect(reclaimableMetric().text).toContain("释放空间是估算值");
  });

  it("renders the same potential semantics in English", async () => {
    configureHealth({ analysisRuns: [cleanupRun(2, 0, 8192)] });
    await renderOverview(englishChrome);

    const metric = reclaimableMetric(en("overviewMetricReclaimable"));
    expect(metric.value).toBe("~8.0 KB");
    expect(metric.hint).toBe("Free space is an estimate");
    expect(metric.text).toContain("Free space is an estimate");
  });

  it("renders no-data for both no cleanup run and zero reclaimable bytes", async () => {
    configureHealth();
    await renderOverview();
    expect(reclaimableMetric()).toMatchObject({ value: "—", hint: "" });

    act(() => root.unmount());
    container.remove();
    resetStores();
    configureHealth({ analysisRuns: [cleanupRun(2, 0, 0)] });
    await renderOverview();
    expect(reclaimableMetric()).toMatchObject({ value: "—", hint: "" });
  });

  it("uses only durable cleanup counts when the backend returns a run", async () => {
    const run = cleanupRun(2, 2048);
    configureHealth({ analysisRuns: [run] });
    await renderOverview();
    expect(priorityTitle()).toBe("有 2 项清理候选");
  });

  it("does not show cleanup when durable cleanup bytes are zero", async () => {
    const run = cleanupRun(2, 0, 0);
    configureHealth({ analysisRuns: [run] });
    await renderOverview();
    expect(priorityTitle()).toBe("文件空间保持有序");
  });

  it("uses potential cleanup bytes as an estimate only when exact bytes are zero", async () => {
    const run = cleanupRun(2, 0, 8192);
    configureHealth({ analysisRuns: [run] });
    await renderOverview();
    expect(priorityTitle()).toBe("有 2 项清理候选");
    expect(container.textContent).toContain("预计可释放 8.0 KB");
  });

  it("does not show a cleanup task when both durable byte totals are zero", async () => {
    const run = cleanupRun(2, 0, 0);
    configureHealth({ analysisRuns: [run] });
    await renderOverview();
    expect(priorityTitle()).toBe("文件空间保持有序");
  });

  it("shows a failed Content Run from the durable health snapshot", async () => {
    configureHealth({ contentRuns: [{ status: "failed", updatedAt: 2, lastErrorDetail: "内容处理失败" }] });
    await renderOverview();
    expect(priorityTitle()).toBe("内容理解任务需要复核");
  });

  it("uses safe findings in the durable cleanup candidate total", async () => {
    configureHealth({ analysisRuns: [cleanupRun(0, 4096, 4096, 2)] });
    await renderOverview();
    expect(priorityTitle()).toBe("有 2 项清理候选");
  });

  it("includes safe, review, and caution findings in an estimated cleanup total", async () => {
    configureHealth({ analysisRuns: [cleanupRun(2, 0, 8192, 0, 1)] });
    await renderOverview();
    expect(priorityTitle()).toBe("有 3 项清理候选");
    expect(container.textContent).toContain("预计可释放 8.0 KB");
  });

  it("does not show cleanup when durable findings are all zero even if bytes are positive", async () => {
    configureHealth({ analysisRuns: [cleanupRun(0, 8192, 8192)] });
    await renderOverview();
    expect(priorityTitle()).toBe("文件空间保持有序");
  });

  it("uses only the newest Content Run status", async () => {
    configureHealth({ contentRuns: [
      { status: "failed", updatedAt: 1, lastErrorDetail: "旧失败" },
      { status: "completed", updatedAt: 2, lastErrorDetail: null }
    ] });
    await renderOverview();
    expect(priorityTitle()).toBe("文件空间保持有序");

    act(() => root.unmount());
    container.remove();
    resetStores();
    configureHealth({ contentRuns: [
      { status: "completed", updatedAt: 1, lastErrorDetail: null },
      { status: "failed", updatedAt: 2, lastErrorDetail: "新失败" }
    ] });
    await renderOverview();
    expect(priorityTitle()).toBe("内容理解任务需要复核");
  });

  it("keeps Content Run status stable for reversed input, empty history, and a latest running run", async () => {
    configureHealth({ contentRuns: [
      { status: "completed", updatedAt: 2, lastErrorDetail: null },
      { status: "failed", updatedAt: 1, lastErrorDetail: "旧失败" }
    ] });
    await renderOverview();
    expect(priorityTitle()).toBe("文件空间保持有序");

    act(() => root.unmount());
    container.remove();
    resetStores();
    configureHealth();
    await renderOverview();
    expect(priorityTitle()).toBe("文件空间保持有序");

    act(() => root.unmount());
    container.remove();
    resetStores();
    configureHealth({ contentRuns: [
      { status: "failed", updatedAt: 1, lastErrorDetail: "旧失败" },
      { status: "running", updatedAt: 2, lastErrorDetail: null }
    ] });
    await renderOverview();
    expect(priorityTitle()).toBe("文件空间保持有序");
  });

  it("derives indexNeedsUpdate from the real Global Index status", async () => {
    configureHealth({ index: { status: "partial", collectionComplete: false, indexedVolumes: 1, readyVolumes: 0, totalEntries: 20 } });
    await renderOverview();
    expect(priorityTitle()).toBe("本地索引可以更新");
  });
});
