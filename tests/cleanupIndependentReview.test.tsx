// @vitest-environment happy-dom

import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import type { AIProductFeatureReadiness, AnalysisFinding, AnalysisRun } from "../src/types/domain";
import { reconcileAuthoritativeFindingUpdates, StorageCleanupView } from "../src/views/cleanup/StorageCleanupView";
import { WorkflowSteps } from "../src/views/shared/ui";

const dialogMocks = vi.hoisted(() => ({ open: vi.fn() }));
const pathMocks = vi.hoisted(() => ({
  desktopDir: vi.fn(),
  documentDir: vi.fn(),
  downloadDir: vi.fn(),
  tempDir: vi.fn()
}));

vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);
vi.mock("@tauri-apps/api/path", () => pathMocks);

const t = makeTranslator("zh");
let root: Root;
let container: HTMLDivElement;

const CleanupView = StorageCleanupView as unknown as (props: Record<string, unknown>) => ReactElement;

function makeRun(id: string, status: string, reviewCount = 0, options: { paths?: string[]; safeCount?: number } = {}): AnalysisRun {
  const paths = options.paths ?? ["C:/Root"];
  const safeCount = options.safeCount ?? 0;
  const findingsCount = reviewCount + safeCount;
  return {
    id,
    requestKey: `request-${id}`,
    requestAttempt: 1,
    scope: { kind: "approved_cleanup_paths", paths },
    scopeHash: `scope-${id}`,
    sourceSnapshot: {},
    sourceSnapshotHash: `snapshot-${id}`,
    detectorSet: ["cleanup_heuristics_v1:v1"],
    detectorSetHash: `detectors-${id}`,
    status,
    phase: ["completed", "partial", "completed_with_warnings", "completed_partial", "cancelled", "canceled", "failed", "error"].includes(status) ? "completed" : "running_detectors",
    revision: 2,
    cancelRequested: false,
    rerunRequired: false,
    detectorsTotal: 1,
    detectorsCompleted: status === "completed" ? 1 : 0,
    detectorsFailed: 0,
    findingsStaged: findingsCount,
    findingsPublished: findingsCount,
    safeCount,
    reviewCount,
    cautionCount: 0,
    exactReclaimableBytes: 0,
    potentialReclaimableBytes: findingsCount,
    warningCount: 0,
    errorCount: 0,
    startedAt: 1,
    finishedAt: status === "completed" ? 2 : null,
    lastCheckpointAt: 2,
    createdAt: 1,
    updatedAt: 2,
    errorCode: null,
    errorMessage: null
  };
}

function makeFinding(run: AnalysisRun, index: number): AnalysisFinding {
  const rootPath = (Array.isArray(run.scope.paths) ? run.scope.paths[0] : "C:/Root") as string;
  return {
    id: `finding-${index}`,
    findingKey: `finding-key-${index}`,
    runId: run.id,
    detectorId: "cleanup_heuristics_v1",
    detectorVersion: 1,
    scopeHash: run.scopeHash,
    status: "active",
    tier: "review",
    category: "review",
    actionKind: "safe_trash_candidate",
    title: `Review ${index}`,
    reason: "Needs confirmation",
    riskNote: null,
    confidence: "estimated",
    sizeBytes: 1,
    exactReclaimableBytes: null,
    potentialReclaimableBytes: 1,
    requiresConfirmation: true,
    executable: true,
    primarySubjectKind: "approved_path",
    primarySubjectId: `${rootPath}/item-${index}`,
    pathSnapshot: `${rootPath}/item-${index}`,
    identitySnapshot: {},
    evidenceSummary: {},
    revision: 1,
    createdAt: 1,
    updatedAt: 1,
    publishedAt: 1,
    staleAt: null,
    decision: null,
    snoozedUntil: null,
    decisionRevision: null
  };
}

function makeAcknowledgedFinding(run: AnalysisRun, index: number): AnalysisFinding {
  return {
    ...makeFinding(run, index),
    decision: "acknowledged",
    decisionRevision: 1
  };
}

function makeCautionFinding(finding: AnalysisFinding, revision = 2): AnalysisFinding {
  return {
    ...finding,
    tier: "caution",
    category: "caution",
    title: `Caution ${finding.id}`,
    requiresConfirmation: false,
    revision,
    updatedAt: 2
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function makeSafeFinding(run: AnalysisRun, index: number): AnalysisFinding {
  return {
    ...makeFinding(run, index),
    tier: "safe",
    category: "safe",
    title: `Safe ${index}`,
    requiresConfirmation: false
  };
}

function readyAIReadiness(): AIProductFeatureReadiness {
  const provider = {
    state: "ready" as const,
    reason: "provider_configuration_ready",
    providerMode: "local" as const,
    providerKind: "ollama" as const,
    providerPreset: null,
    model: "fixture-model",
    credentialRequired: false,
    credentialConfigured: false,
    settingsRevision: "test-fixture",
    bindingFingerprint: "test-fixture"
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
      reason: "managed_ai_ready",
      provider,
      managedScopeId: "fixture-scope",
      scopeFingerprint: "test-fixture",
      bindingFingerprint: "test-fixture",
      disclosure
    }],
    cleanup: {
      state: "ready",
      reason: "cleanup_ai_ready",
      provider,
      cleanupAiEnabled: true,
      localAiAllowed: true,
      cloudAiAllowed: false,
      bindingFingerprint: "test-fixture",
      disclosure
    }
  };
}

function scopeButton(): HTMLButtonElement {
  const section = container.querySelector<HTMLElement>("[data-cleanup-scope]");
  const found = [...(section?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((button) => /扫描|重新扫描/.test(button.textContent ?? ""));
  if (!found) throw new Error("scope scan button not found");
  return found;
}

function button(text: string): HTMLButtonElement {
  const found = [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.includes(text));
  if (!found) throw new Error(`button not found: ${text}`);
  return found;
}

function findingButton(findingId: string, text: string): HTMLButtonElement {
  const row = container.querySelector<HTMLElement>(`[data-analysis-finding-id="${findingId}"]`);
  const found = [...(row?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((item) => item.textContent?.includes(text));
  if (!found) throw new Error(`finding button not found: ${findingId} / ${text}; row=${row?.textContent ?? "missing"}`);
  return found;
}

function reactOnClick(button: HTMLButtonElement): (() => void) | null {
  const propsKey = Object.keys(button).find((key) => key.startsWith("__reactProps$"));
  if (!propsKey) return null;
  const props = (button as unknown as Record<string, { onClick?: unknown }>)[propsKey];
  return typeof props?.onClick === "function" ? props.onClick as () => void : null;
}

async function flush(count = 3) {
  for (let index = 0; index < count; index += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
}

function commonApi(run: AnalysisRun, overrides: Record<string, unknown> = {}) {
  return {
    listAnalysisDetectors: async () => [],
    startAnalysisRun: async () => run,
    getActiveAnalysisRun: async () => null,
    listAnalysisRuns: async () => [],
    getAnalysisRun: async () => run,
    listAnalysisRunDetectors: async () => [],
    listAnalysisFindings: async () => ({ findings: [], nextCursor: null, limit: 100 }),
    getAIFeatureReadiness: async () => readyAIReadiness(),
    onAnalysisRunUpdated: async () => () => undefined,
    onAnalysisFindingsPublished: async () => () => undefined,
    onAnalysisDetectorUpdated: async () => () => undefined,
    ...overrides
  } as any;
}

describe("Cleanup independent review behavior", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState({}, "", "/");
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    document.body.innerHTML = '<div id="test-root"></div>';
    container = document.getElementById("test-root") as HTMLDivElement;
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    document.body.innerHTML = "";
  });

  async function renderDurableRun(run: AnalysisRun) {
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run
    });
    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/Root"], api, t })));
    await flush(8);
  }

  it("renders workflow progress with native ordered-list semantics and accessible state labels", () => {
    const states = [
      { label: "扫描", state: "done" as const, stateLabel: "已完成" },
      { label: "复核", state: "current" as const, stateLabel: "当前步骤" },
      { label: "安全清理", state: "blocked" as const, stateLabel: "需要处理" },
      { label: "恢复", state: "pending" as const, stateLabel: "待处理" }
    ];
    const wrapper = document.createElement("div");
    wrapper.innerHTML = renderToStaticMarkup(createElement(WorkflowSteps, { label: "清理工作流", steps: states }));

    const workflow = wrapper.querySelector<HTMLOListElement>("[data-workflow-steps]");
    expect(workflow?.tagName).toBe("OL");
    expect(workflow?.getAttribute("role")).toBeNull();
    const items = workflow ? [...workflow.children] : [];
    expect(items.map((item) => item.tagName)).toEqual(["LI", "LI", "LI", "LI"]);
    expect(items.every((item) => item.getAttribute("role") === null)).toBe(true);
    expect(items.map((item) => item.getAttribute("data-workflow-state"))).toEqual(states.map((step) => step.state));
    expect(items.map((item) => item.getAttribute("aria-label"))).toEqual(states.map((step) => `${step.stateLabel}: ${step.label}`));
    expect(items.map((item) => item.querySelector<HTMLElement>("[data-workflow-state-label]")?.textContent)).toEqual(states.map((step) => step.stateLabel));
    expect(items.every((item) => item.querySelector<HTMLElement>("[data-workflow-marker]")?.getAttribute("aria-hidden") === "true")).toBe(true);
    expect(items[1]?.getAttribute("aria-current")).toBe("step");
    expect(items[0]?.getAttribute("aria-current")).toBeNull();
  });

  it("treats a completed empty analysis as a valid empty result, not blocked", async () => {
    await renderDurableRun(makeRun("run-completed-empty", "completed"));

    expect(container.textContent).toContain(t("storageCleanupRunCompletedEmptyTitle"));
    expect(container.textContent).toContain(t("storageCleanupRunCompletedEmptyDesc"));
    expect(container.querySelector<HTMLElement>("[data-workflow-steps] [data-workflow-state]")?.dataset.workflowState).toBe("done");
    expect(container.querySelector('[data-workflow-state="blocked"]')).toBeNull();
  });

  it("preserves partial truth for a partial empty analysis", async () => {
    await renderDurableRun(makeRun("run-partial-empty", "partial"));

    expect(container.textContent).toContain(t("storageCleanupRunPartialEmptyTitle"));
    expect(container.textContent).toContain(t("storageCleanupRunPartialEmptyDesc"));
    expect(container.querySelector<HTMLElement>("[data-workflow-steps] [data-workflow-state]")?.dataset.workflowState).toBe("done");
    expect(container.querySelector('[data-workflow-state="blocked"]')).toBeNull();
  });

  it("preserves canceled truth for a canceled empty analysis", async () => {
    await renderDurableRun(makeRun("run-canceled-empty", "cancelled"));

    expect(container.textContent).toContain(t("storageCleanupRunCanceledEmptyTitle"));
    expect(container.textContent).toContain(t("storageCleanupRunCanceledEmptyDesc"));
    expect(container.querySelector<HTMLElement>("[data-workflow-steps] [data-workflow-state]")?.dataset.workflowState).toBe("done");
    expect(container.querySelector('[data-workflow-state="blocked"]')).toBeNull();
  });

  it("preserves failed truth independently of the published finding count", async () => {
    const failedRun = { ...makeRun("run-failed-with-findings", "failed", 1), errorCount: 1, errorMessage: "detector_failed" };
    await renderDurableRun(failedRun);

    expect(container.textContent).toContain(t("storageCleanupRunFailed"));
    expect(container.textContent).not.toContain(t("storageCleanupRunPartialTitle"));
    expect(container.querySelector<HTMLElement>("[data-workflow-steps] [data-workflow-state]")?.dataset.workflowState).toBe("blocked");
    expect(container.querySelectorAll('[data-workflow-steps] [data-workflow-state="blocked"]')).toHaveLength(1);
    expect(container.querySelector('[data-workflow-steps] [data-workflow-state="blocked"]')?.getAttribute("aria-label")).toContain(t("workflowStateBlocked"));
  });

  it("creates one request key per scan intent and blocks duplicate submission", async () => {
    const runA = makeRun("run-a", "completed");
    const runB = makeRun("run-b", "completed");
    const requests: Array<{ requestKey?: string | null }> = [];
    const startAnalysisRun = vi.fn(async (request: { requestKey?: string | null }) => {
      requests.push(request);
      return requests.length === 1 ? runA : runB;
    });
    const api = commonApi(runA, {
      startAnalysisRun,
      getAnalysisRun: vi.fn(async (id: string) => id === runA.id ? runA : runB)
    });

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/Root"], api, t })));
    await flush();
    await act(async () => {
      scopeButton().click();
      scopeButton().click();
    });
    await flush();
    expect(startAnalysisRun).toHaveBeenCalledOnce();
    expect(requests[0]?.requestKey).toMatch(/^cleanup-/);

    await act(async () => scopeButton().click());
    await flush();
    expect(startAnalysisRun).toHaveBeenCalledTimes(2);
    expect(requests[1]?.requestKey).toMatch(/^cleanup-/);
    expect(requests[1]?.requestKey).not.toBe(requests[0]?.requestKey);
  });

  it("keeps the cleanup scope locked while the start request is pending", async () => {
    const runA = makeRun("run-scan-old", "completed", 0, { paths: ["C:/RootA"] });
    const requests: Array<{ scope?: { paths?: string[] }; requestKey?: string | null }> = [];
    let resolveOld: (run: AnalysisRun) => void = () => undefined;
    const startAnalysisRun = vi.fn((request: { scope?: { paths?: string[] }; requestKey?: string | null }) => {
      requests.push(request);
      return new Promise<AnalysisRun>((resolve) => { resolveOld = resolve; });
    });
    const api = commonApi(runA, {
      listAnalysisRuns: async () => [],
      startAnalysisRun,
      getAnalysisRun: vi.fn(async () => runA)
    });
    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootA"], api, t })));
    await flush(3);
    await act(async () => scopeButton().click());
    await flush(2);
    expect(startAnalysisRun).toHaveBeenCalledOnce();
    expect(button(t("storageCleanupChooseFolder")).disabled).toBe(true);
    expect(scopeButton().disabled).toBe(true);
    resolveOld(runA);
    await flush(8);
    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).not.toBeNull();
    expect(scopeButton().disabled).toBe(false);
  });

  it("does not let a settled scan finally clear a newer cancel mutation lock", async () => {
    const running = makeRun("run-scan-cancel-owner", "running");
    const canceled = { ...running, status: "cancelled", phase: "completed" as const };
    let resolveStartDetails: (run: AnalysisRun) => void = () => undefined;
    let resolveCancelDetails: (run: AnalysisRun) => void = () => undefined;
    let detailCalls = 0;
    const startAnalysisRun = vi.fn(async () => running);
    const cancelAnalysisRun = vi.fn(async () => canceled);
    const api = commonApi(running, {
      listAnalysisRuns: async () => [],
      startAnalysisRun,
      cancelAnalysisRun,
      getAnalysisRun: vi.fn(() => {
        detailCalls += 1;
        return new Promise<AnalysisRun>((resolve) => {
          if (detailCalls === 1) resolveStartDetails = resolve;
          else resolveCancelDetails = resolve;
        });
      })
    });

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/Root"], api, t })));
    await flush(3);
    await act(async () => scopeButton().click());
    await vi.waitFor(() => expect(startAnalysisRun).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(button(t("storageCleanupCancelScan"))).toBeDefined());

    await act(async () => button(t("storageCleanupCancelScan")).click());
    await vi.waitFor(() => expect(cancelAnalysisRun).toHaveBeenCalledOnce());
    expect(scopeButton().disabled).toBe(true);

    resolveStartDetails(running);
    await flush(6);
    expect(scopeButton().disabled).toBe(true);

    resolveCancelDetails(canceled);
    await flush(8);
    expect(scopeButton().disabled).toBe(false);
  });

  it("reports a pending scan failure and releases the cleanup mutation lock", async () => {
    const runA = makeRun("run-scan-error-old", "completed", 0, { paths: ["C:/RootA"] });
    let rejectOld: (error: unknown) => void = () => undefined;
    const startAnalysisRun = vi.fn((request: { scope?: { paths?: string[] }; requestKey?: string | null }) => {
      return new Promise<AnalysisRun>((_, reject) => { rejectOld = reject; });
    });
    const onError = vi.fn();
    const api = commonApi(runA, {
      listAnalysisRuns: async () => [],
      startAnalysisRun,
      getAnalysisRun: vi.fn(async () => runA)
    });
    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootA"], api, t, onError })));
    await flush(3);
    await act(async () => scopeButton().click());
    await flush(2);
    expect(startAnalysisRun).toHaveBeenCalledOnce();
    expect(scopeButton().disabled).toBe(true);

    rejectOld(new Error("scope_a_failed"));
    await flush(8);

    expect(onError).toHaveBeenCalledWith(expect.stringContaining("scope_a_failed"));
    expect(scopeButton().disabled).toBe(false);

    await act(async () => scopeButton().click());
    expect(startAnalysisRun).toHaveBeenCalledTimes(2);
  });

  it("does not allow a second scan until the first scan settles", async () => {
    const runA = makeRun("run-scan-lock-old", "completed", 0, { paths: ["C:/RootA"] });
    const runB = makeRun("run-scan-lock-new", "completed", 0, { paths: ["C:/RootB"] });
    const requests: Array<{ scope?: { paths?: string[] }; requestKey?: string | null }> = [];
    let rejectOld: (error: unknown) => void = () => undefined;
    let resolveNew: (run: AnalysisRun) => void = () => undefined;
    const startAnalysisRun = vi.fn((request: { scope?: { paths?: string[] }; requestKey?: string | null }) => {
      requests.push(request);
      if (requests.length === 1) return new Promise<AnalysisRun>((_, reject) => { rejectOld = reject; });
      return new Promise<AnalysisRun>((resolve) => { resolveNew = resolve; });
    });
    const onError = vi.fn();
    const api = commonApi(runA, {
      listAnalysisRuns: async () => [],
      startAnalysisRun,
      getAnalysisRun: vi.fn(async (id: string) => id === runB.id ? runB : runA)
    });
    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootA"], api, t, onError })));
    await flush(3);
    await act(async () => scopeButton().click());
    await flush(2);
    expect(startAnalysisRun).toHaveBeenCalledOnce();
    expect(scopeButton().disabled).toBe(true);
    rejectOld(new Error("scope_a_failed"));
    await flush(6);
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("scope_a_failed"));
    expect(scopeButton().disabled).toBe(false);

    await act(async () => scopeButton().click());
    await flush(3);
    expect(startAnalysisRun).toHaveBeenCalledTimes(2);
    expect(scopeButton().disabled).toBe(true);
    resolveNew(runB);
    await flush(8);
    expect(container.querySelector(`[data-analysis-run-id="${runB.id}"]`)).not.toBeNull();
    expect(scopeButton().disabled).toBe(false);
  });

  it("uses a fresh request key after a canceled run", async () => {
    const running = makeRun("run-running", "running");
    const canceled = { ...running, status: "cancelled", phase: "completed" as const, cancelRequested: false };
    const fresh = makeRun("run-fresh", "completed");
    const requests: Array<{ requestKey?: string | null }> = [];
    let canceledAt = false;
    const startAnalysisRun = vi.fn(async (request: { requestKey?: string | null }) => {
      requests.push(request);
      return requests.length === 1 ? running : fresh;
    });
    const cancelAnalysisRun = vi.fn(async () => {
      canceledAt = true;
      return canceled;
    });
    const api = commonApi(running, {
      startAnalysisRun,
      cancelAnalysisRun,
      getAnalysisRun: vi.fn(async (id: string) => id === running.id ? (canceledAt ? canceled : running) : fresh)
    });

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/Root"], api, t })));
    await flush();
    await act(async () => scopeButton().click());
    await flush();
    await act(async () => button("取消扫描").click());
    await flush();
    await act(async () => scopeButton().click());
    await flush();
    expect(cancelAnalysisRun).toHaveBeenCalledOnce();
    expect(requests).toHaveLength(2);
    expect(requests[0]?.requestKey).not.toBe(requests[1]?.requestKey);
  });

  it("collects and batches the complete Review tier instead of only loaded findings", async () => {
    const run = makeRun("run-ai", "completed", 205);
    const allIds = Array.from({ length: 205 }, (_, index) => `finding-${index}`);
    const listAnalysisFindings = vi.fn(async (request: { tier?: string; cursor?: string | null }) => {
      if (request.tier !== "review") return { findings: [], nextCursor: null, limit: 100 };
      const offset = Number(request.cursor ?? 0);
      const pageIds = allIds.slice(offset, offset + 100);
      const next = offset + pageIds.length < allIds.length ? String(offset + pageIds.length) : null;
      return { findings: pageIds.map((_, index) => makeFinding(run, offset + index)), nextCursor: next, limit: 100 };
    });
    const analyzeCleanupCandidatesWithAI = vi.fn(async (_runId: string, _findingIds: string[]) => []);
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      listAnalysisFindings,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush();
    await act(async () => button("需人工判断").click());
    await flush();
    await act(async () => button("重新核验需确认项目").click());
    await flush(8);

    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledTimes(5);
    const submitted = analyzeCleanupCandidatesWithAI.mock.calls.flatMap((call) => call[1] as string[]);
    expect(submitted).toHaveLength(205);
    expect(new Set(submitted)).toEqual(new Set(allIds));
    expect(container.textContent).toContain("共 205 项");
  });

  it("stops an in-flight AI recheck without starting another batch", async () => {
    const run = makeRun("run-ai-cancel", "completed", 60);
    const findings = Array.from({ length: 60 }, (_, index) => makeFinding(run, index));
    let resolveBatch: (value: AnalysisFinding[]) => void = () => undefined;
    const analyzeCleanupCandidatesWithAI = vi.fn(() => new Promise<AnalysisFinding[]>((resolve) => { resolveBatch = resolve; }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      listAnalysisFindings: async () => ({ findings, nextCursor: null, limit: 100 }),
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush();
    await act(async () => button("需人工判断").click());
    await flush();
    const staleStartHandler = reactOnClick(button("重新核验需确认项目"));
    await act(async () => button("重新核验需确认项目").click());
    await flush(5);
    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();

    await act(async () => button("停止重新核验").click());
    expect(button(t("storageCleanupAIRecheckCanceling")).disabled).toBe(true);
    await act(async () => staleStartHandler?.());
    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();
    resolveBatch([]);
    await flush(5);

    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("重新核验已停止");
    expect(container.textContent).not.toContain("停止重新核验");

    await act(async () => button("重新核验需确认项目").click());
    await flush(5);
    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledTimes(2);
    resolveBatch([]);
    await flush(5);
  });

  it("reconciles a successful settled batch after cancellation before reloading the tier", async () => {
    const run = makeRun("run-ai-cancel-settled-success", "completed", 1);
    const selected = makeAcknowledgedFinding(run, 0);
    const updated = makeCautionFinding(selected);
    const updatedRun = { ...run, revision: 3, reviewCount: 0, cautionCount: 1, updatedAt: 3 };
    let currentRun = run;
    const batch = deferred<AnalysisFinding[]>();
    const analyzeCleanupCandidatesWithAI = vi.fn(() => batch.promise);
    const getAnalysisRun = vi.fn(async () => currentRun);
    const listAnalysisFindings = vi.fn(async (request: { tier?: string }) => ({
      findings: request.tier === "review" && currentRun.id === run.id && currentRun.revision < updatedRun.revision ? [selected] : [],
      nextCursor: null,
      limit: 100
    }));
    const getAnalysisFinding = vi.fn(async () => updated);
    const previewCleanupOperations = vi.fn(async () => ({ total: 0, previews: [], truncated: false, hasMore: false }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun,
      listAnalysisFindings,
      getAnalysisFinding,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI,
      previewCleanupOperations
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await vi.waitFor(() => expect(findingButton(selected.id, t("storageCleanupSelectForTrash"))).toBeDefined());
    await act(async () => findingButton(selected.id, t("storageCleanupSelectForTrash")).click());
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();

    await act(async () => button("重新核验需确认项目").click());
    await vi.waitFor(() => expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce());
    await act(async () => button("停止重新核验").click());

    currentRun = updatedRun;
    await act(async () => batch.resolve([updated]));
    await flush(20);

    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();
    expect(getAnalysisRun.mock.calls.length).toBeGreaterThan(1);
    expect(getAnalysisFinding).toHaveBeenCalledWith(selected.id);
    expect(getAnalysisFinding).toHaveBeenCalledTimes(1);
    const reviewRequests = listAnalysisFindings.mock.calls.filter(([request]) => request.tier === "review");
    expect(reviewRequests.length).toBeGreaterThan(1);
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).toContain("重新核验已停止：已处理 1/1 项");
    expect(container.textContent).not.toContain("重新核验完成");
    expect(previewCleanupOperations).not.toHaveBeenCalled();
  });

  it("reconciles a partially persisted provider failure after cancellation", async () => {
    const run = makeRun("run-ai-cancel-partial-failure", "completed", 2);
    const first = makeAcknowledgedFinding(run, 0);
    const second = makeAcknowledgedFinding(run, 1);
    const updatedFirst = makeCautionFinding(first);
    const updatedRun = { ...run, revision: 3, reviewCount: 1, cautionCount: 1, updatedAt: 3 };
    let currentRun = run;
    const batch = deferred<AnalysisFinding[]>();
    const analyzeCleanupCandidatesWithAI = vi.fn(() => batch.promise);
    const listAnalysisFindings = vi.fn(async (request: { tier?: string }) => ({
      findings: request.tier === "review" ? (currentRun.revision < updatedRun.revision ? [first, second] : [second]) : [],
      nextCursor: null,
      limit: 100
    }));
    const getAnalysisFinding = vi.fn(async (findingId: string) => {
      if (findingId === first.id) return updatedFirst;
      throw new Error("readback_failed");
    });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: vi.fn(async () => currentRun),
      listAnalysisFindings,
      getAnalysisFinding,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await vi.waitFor(() => expect(findingButton(first.id, t("storageCleanupSelectForTrash"))).toBeDefined());
    await act(async () => findingButton(first.id, t("storageCleanupSelectForTrash")).click());
    await act(async () => findingButton(second.id, t("storageCleanupSelectForTrash")).click());
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();

    await act(async () => button("重新核验需确认项目").click());
    await vi.waitFor(() => expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce());
    await act(async () => button("停止重新核验").click());

    currentRun = updatedRun;
    await act(async () => batch.reject(new Error("provider_failed_after_cancel")));
    await flush(20);

    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();
    expect(getAnalysisFinding).toHaveBeenCalledWith(first.id);
    expect(getAnalysisFinding).toHaveBeenCalledWith(second.id);
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).toContain("重新核验已停止：已处理 0/2 项，跳过 0，AI 更新失败 2，权威读回失败 1");
    expect(container.textContent).not.toContain("重新核验完成");
  });

  it("invalidates selection and cache when a canceled batch readback fails", async () => {
    const run = makeRun("run-ai-cancel-readback-failure", "completed", 1);
    const selected = makeAcknowledgedFinding(run, 0);
    const updatedRun = { ...run, revision: 3, reviewCount: 0, updatedAt: 3 };
    let currentRun = run;
    const batch = deferred<AnalysisFinding[]>();
    const analyzeCleanupCandidatesWithAI = vi.fn(() => batch.promise);
    const listAnalysisFindings = vi.fn(async (request: { tier?: string }) => ({
      findings: request.tier === "review" && currentRun.revision < updatedRun.revision ? [selected] : [],
      nextCursor: null,
      limit: 100
    }));
    const getAnalysisFinding = vi.fn(async () => {
      throw new Error("readback_failed");
    });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: vi.fn(async () => currentRun),
      listAnalysisFindings,
      getAnalysisFinding,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await vi.waitFor(() => expect(findingButton(selected.id, t("storageCleanupSelectForTrash"))).toBeDefined());
    await act(async () => findingButton(selected.id, t("storageCleanupSelectForTrash")).click());
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();

    await act(async () => button("重新核验需确认项目").click());
    await vi.waitFor(() => expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce());
    await act(async () => button("停止重新核验").click());

    currentRun = updatedRun;
    await act(async () => batch.resolve([selected]));
    await flush(20);

    expect(getAnalysisFinding).toHaveBeenCalledWith(selected.id);
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).toContain("权威读回失败 1");
    expect(container.textContent).not.toContain("重新核验完成");
  });

  it("keeps the same current Cleanup finding reviewable and previewable after a provider request fails", async () => {
    const run = makeRun("run-ai-request-failure-current-assessment", "completed", 1);
    const review = makeFinding(run, 0);
    const acknowledged = { ...review, decision: "acknowledged" as const, decisionRevision: 2 };
    const readinessSnapshot = readyAIReadiness();
    const readinessBindings: string[] = [];
    const getAIFeatureReadiness = vi.fn(async () => {
      readinessBindings.push([
        readinessSnapshot.provider.settingsRevision,
        readinessSnapshot.provider.bindingFingerprint,
        readinessSnapshot.cleanup.bindingFingerprint
      ].join(":"));
      return readinessSnapshot;
    });
    let decisionSaved = false;
    const analyzeCleanupCandidatesWithAI = vi.fn(async () => {
      throw new Error("provider_request_unavailable");
    });
    const previewCleanupOperations = vi.fn(async (_runId: string, selections: Array<{ findingId: string }>) => {
      expect(selections.map((selection) => selection.findingId)).toEqual([review.id]);
      return {
      total: 1,
      previews: [{
        id: "preview-after-provider-request-failure",
        fileId: review.id,
        operation_type: "move_to_trash" as const,
        source_path: review.pathSnapshot ?? "C:/Root/item-0",
        target_path: "Recycle Bin",
        old_name: "item-0",
        new_name: "item-0",
        status: "pending" as const,
        risk_level: "Normal" as const,
        confidence: 1,
        requires_confirmation: true,
        reason: "Mounted test fixture for deterministic Preview presentation",
        is_executable: true
      }],
      limit: 100,
      offset: 0,
      truncated: false,
      hasMore: false
      };
    });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      getAIFeatureReadiness,
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "review" ? [decisionSaved ? acknowledged : review] : [],
        nextCursor: null,
        limit: 100
      }),
      getAnalysisFinding: vi.fn(async () => decisionSaved ? acknowledged : review),
      setAnalysisFindingDecision: vi.fn(async () => {
        decisionSaved = true;
        return { decision: "acknowledged", revision: 2 };
      }),
      listAnalysisFindingEvidence: async () => [{
        id: "current-ai-evidence",
        findingId: review.id,
        evidenceKind: "ai_assessment",
        subjectKind: "analysis_finding",
        subjectId: review.id,
        pathSnapshot: review.pathSnapshot,
        value: { schema: "cleanup_ai_assessment_v1" },
        createdAt: 1,
        isCurrentAssessment: true
      }],
      analyzeCleanupCandidatesWithAI,
      previewCleanupOperations
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(4);
    await act(async () => findingButton(review.id, t("storageCleanupFindingEvidence")).click());
    await flush(4);
    expect(container.querySelector('[data-ai-assessment-current="true"]')).not.toBeNull();

    await act(async () => button(t("storageCleanupAIRecheck")).click());
    await flush(12);
    expect(getAIFeatureReadiness).toHaveBeenCalledTimes(2);
    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();
    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledWith(run.id, [review.id]);
    expect(container.textContent).toContain("AI 更新失败 1");
    expect(container.querySelector(`[data-analysis-finding-id="${review.id}"]`)).not.toBeNull();
    expect(container.querySelector('[data-ai-assessment-current="true"]')).not.toBeNull();
    expect(readinessBindings).toEqual([
      "test-fixture:test-fixture:test-fixture",
      "test-fixture:test-fixture:test-fixture"
    ]);

    await act(async () => findingButton(review.id, t("storageCleanupFindingAcknowledge")).click());
    await flush(4);
    await act(async () => button(t("storageCleanupReviewConfirmAction")).click());
    await flush(8);
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await vi.waitFor(() => expect(previewCleanupOperations).toHaveBeenCalledOnce());
    await flush(5);
    expect(container.querySelector('[data-cleanup-preview-state="ready"]')).not.toBeNull();
    expect(container.textContent).toContain(t("storageCleanupPreviewReadyTitle"));
    expect(previewCleanupOperations).toHaveBeenCalledWith(run.id, [{
      findingId: review.id,
      expectedRevision: review.revision,
      reviewConfirmation: { decisionRevision: acknowledged.decisionRevision }
    }]);
    expect(container.querySelector('[data-ai-assessment-current="true"]')).not.toBeNull();
  });

  it("keeps output-name Review findings reveal-only even after acknowledgement", async () => {
    const run = makeRun("run-output-names", "completed", 2);
    const findings = ["build", "dist"].map((name, index): AnalysisFinding => ({
      ...makeAcknowledgedFinding(run, index),
      title: name,
      pathSnapshot: `F:/Coding/Zen-Canvas/Library/${name}`,
      actionKind: "reveal",
      executable: false,
      confidence: "estimated"
    }));
    const revealInFolder = vi.fn(async () => undefined);
    const previewCleanupOperations = vi.fn();
    const moveCleanupCandidatesToSafeTrash = vi.fn();
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async ({ tier }: { tier?: string }) => ({
        findings: tier === "review" ? findings : [], nextCursor: null, limit: 100
      }),
      revealInFolder,
      previewCleanupOperations,
      moveCleanupCandidatesToSafeTrash
    });
    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button(t("storageCleanupReviewTier")).click());
    await flush(4);
    for (const finding of findings) {
      const row = container.querySelector(`[data-analysis-finding-id="${finding.id}"]`);
      expect(row).not.toBeNull();
      expect(row?.textContent).toContain(t("storageCleanupConfidenceEstimated"));
      expect(row?.textContent).not.toContain(t("storageCleanupSelected"));
      expect(row?.textContent).not.toContain(t("storageCleanupFindingAcknowledge"));
      expect(findingButton(finding.id, t("storageCleanupSelectForTrash")).disabled).toBe(true);
    }
    const reveal = findingButton(findings[0].id, t("storageCleanupReveal"));
    await act(async () => reveal.click());
    expect(revealInFolder).toHaveBeenCalledOnce();
    expect(previewCleanupOperations).not.toHaveBeenCalled();
    expect(moveCleanupCandidatesToSafeTrash).not.toHaveBeenCalled();
  });

  it("keeps deterministic findings inspectable when provider readiness blocks new Cleanup analysis", async () => {
    const run = makeRun("run-ai-provider-blocked", "completed", 1);
    const review = makeFinding(run, 0);
    const disabledProvider = {
      ...readyAIReadiness().provider,
      state: "disabled" as const,
      reason: "provider_disabled"
    };
    const blockedReadiness: AIProductFeatureReadiness = {
      provider: disabledProvider,
      managedScopes: [],
      cleanup: {
        ...readyAIReadiness().cleanup,
        state: "disabled",
        reason: "provider_disabled",
        provider: disabledProvider,
        cleanupAiEnabled: false
      }
    };
    const analyzeCleanupCandidatesWithAI = vi.fn(async () => [] as AnalysisFinding[]);
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      getAIFeatureReadiness: async () => blockedReadiness,
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "review" ? [review] : [],
        nextCursor: null,
        limit: 100
      }),
      listAnalysisFindingEvidence: async () => [{
        id: "deterministic-cleanup-evidence",
        findingId: review.id,
        evidenceKind: "cleanup_heuristic",
        subjectKind: "analysis_finding",
        subjectId: review.id,
        pathSnapshot: review.pathSnapshot,
        value: { detector: "cleanup_heuristics_v1" },
        createdAt: 1,
        isCurrentAssessment: false
      }],
      analyzeCleanupCandidatesWithAI
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button(t("storageCleanupReviewTier")).click());
    await flush(4);

    expect(container.querySelector('[data-ai-cleanup-state="disabled"]')).not.toBeNull();
    expect(container.querySelector(`[data-analysis-finding-id="${review.id}"]`)).not.toBeNull();
    const recheck = button(t("storageCleanupAIRecheck"));
    expect(recheck.disabled).toBe(true);
    await act(async () => findingButton(review.id, t("storageCleanupFindingEvidence")).click());
    await flush(3);
    expect(container.querySelector("[data-finding-evidence]")?.textContent).toContain("cleanup_heuristic");
    expect(analyzeCleanupCandidatesWithAI).not.toHaveBeenCalled();
  });

  it("reconciles the settled first batch and never starts a second batch after cancellation", async () => {
    const run = makeRun("run-ai-cancel-no-second-batch", "completed", 60);
    const findings = Array.from({ length: 60 }, (_, index) => makeAcknowledgedFinding(run, index));
    const batch = deferred<AnalysisFinding[]>();
    const analyzeCleanupCandidatesWithAI = vi.fn(() => batch.promise);
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "review" ? findings : [],
        nextCursor: null,
        limit: 100
      }),
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await vi.waitFor(() => expect(findingButton(findings[0].id, t("storageCleanupSelectForTrash"))).toBeDefined());
    await act(async () => button("重新核验需确认项目").click());
    await vi.waitFor(() => expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce());
    await act(async () => button("停止重新核验").click());
    await act(async () => batch.resolve([]));
    await flush(20);

    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("重新核验已停止");
    expect(container.textContent).not.toContain("重新核验完成");
  });

  it("keeps an AI operation owned by the same run when the durable run revision advances", async () => {
    const run = makeRun("run-ai-self-revision", "completed", 1);
    const selected = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    let currentRun = run;
    let resolveAI: (value: AnalysisFinding[]) => void = () => undefined;
    let onRunUpdate: ((updated: AnalysisRun) => void) | null = null;
    const analyzeCleanupCandidatesWithAI = vi.fn(() => new Promise<AnalysisFinding[]>((resolve) => { resolveAI = resolve; }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: vi.fn(async () => currentRun),
      listAnalysisFindings: async () => ({ findings: [selected], nextCursor: null, limit: 100 }),
      getAnalysisFinding: vi.fn(async () => selected),
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI,
      onAnalysisRunUpdated: async (handler: (updated: AnalysisRun) => void) => {
        onRunUpdate = handler;
        return () => undefined;
      }
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(4);
    await act(async () => button("重新核验需确认项目").click());
    await vi.waitFor(() => expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce());

    currentRun = { ...run, revision: run.revision + 1 };
    await act(async () => onRunUpdate?.(currentRun));
    await flush(6);
    resolveAI([selected]);
    await flush(12);

    expect(api.getAnalysisFinding).toHaveBeenCalledWith(selected.id);
    expect(container.textContent).not.toContain("停止重新核验");
  });

  it("releases AI busy state when cancellation supersedes terminal run refresh", async () => {
    const run = makeRun("run-ai-cancel-refresh", "completed", 1);
    const selected = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    let getRunCalls = 0;
    let resolveTerminalRefresh: (value: AnalysisRun) => void = () => undefined;
    const terminalRefresh = new Promise<AnalysisRun>((resolve) => { resolveTerminalRefresh = resolve; });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: vi.fn(async () => {
        getRunCalls += 1;
        return getRunCalls === 1 ? run : terminalRefresh;
      }),
      listAnalysisFindings: async () => ({ findings: [selected], nextCursor: null, limit: 100 }),
      getAnalysisFinding: vi.fn(async () => selected),
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI: vi.fn(async () => [selected])
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(4);
    await act(async () => button("重新核验需确认项目").click());
    await vi.waitFor(() => expect(getRunCalls).toBe(2));

    await act(async () => button("停止重新核验").click());
    resolveTerminalRefresh(run);
    await flush(8);

    expect([...container.querySelectorAll<HTMLButtonElement>("button")].some((item) => item.textContent?.includes("停止重新核验"))).toBe(false);
    expect([...container.querySelectorAll<HTMLButtonElement>("button")].some((item) => item.textContent?.includes("重新核验需确认项目"))).toBe(true);
  });

  it("keeps a saved finding handler locked during AI and mutation work", async () => {
    const run = makeRun("run-toggle-lock", "completed", 1);
    const finding = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    let resolveAI: (value: AnalysisFinding[]) => void = () => undefined;
    let resolvePreview: (value: { total: number; previews: never[]; truncated: boolean; hasMore: boolean }) => void = () => undefined;
    const analyzeCleanupCandidatesWithAI = vi.fn(() => new Promise<AnalysisFinding[]>((resolve) => { resolveAI = resolve; }));
    const previewCleanupOperations = vi.fn(() => new Promise<{ total: number; previews: never[]; truncated: boolean; hasMore: boolean }>((resolve) => { resolvePreview = resolve; }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      listAnalysisFindings: async () => ({ findings: [finding], nextCursor: null, limit: 100 }),
      getAnalysisFinding: async () => finding,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI,
      previewCleanupOperations
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(4);

    const aiHandler = reactOnClick(findingButton(finding.id, t("storageCleanupSelectForTrash")));
    expect(aiHandler).toBeTypeOf("function");
    await act(async () => button("重新核验需确认项目").click());
    await vi.waitFor(() => expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce());
    await act(async () => aiHandler?.());
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();

    resolveAI([]);
    await flush(10);
    await act(async () => findingButton(finding.id, t("storageCleanupSelectForTrash")).click());
    await flush(2);
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();

    const mutationHandler = reactOnClick(findingButton(finding.id, t("storageCleanupSelected")));
    expect(mutationHandler).toBeTypeOf("function");
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await vi.waitFor(() => expect(previewCleanupOperations).toHaveBeenCalledOnce());
    expect(container.textContent).toContain(t("storageCleanupPreviewLoading"));
    await act(async () => mutationHandler?.());
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();

    resolvePreview({ total: 1, previews: [], truncated: false, hasMore: false });
    await flush(8);
  });

  it("keeps Safe Trash mutation ownership through completion", async () => {
    const run = makeRun("run-safe-trash-lock", "completed", 1);
    const finding = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    let resolveMove: (value: { moved: number; skipped: number; failed: number }) => void = () => undefined;
    const previewCleanupOperations = vi.fn(async () => ({ total: 1, previews: [{
      id: "preview-safe-trash-lock",
      fileId: finding.id,
      operation_type: "move_to_trash" as const,
      source_path: finding.pathSnapshot ?? "C:/Root/item-0",
      target_path: "Recycle Bin",
      old_name: "item-0",
      new_name: "item-0",
      status: "pending" as const,
      risk_level: "Normal" as const,
      confidence: 1,
      requires_confirmation: true,
      reason: "approved",
      is_executable: true
    }], truncated: false, hasMore: false }));
    const moveCleanupCandidatesToSafeTrash = vi.fn(() => new Promise<{ moved: number; skipped: number; failed: number }>((resolve) => { resolveMove = resolve; }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({ findings: request.tier === "review" ? [finding] : [], nextCursor: null, limit: 100 }),
      getAnalysisRun: async () => run,
      previewCleanupOperations,
      moveCleanupCandidatesToSafeTrash
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(finding.id, t("storageCleanupSelectForTrash")).click());
    await flush(3);

    const stalePreviewHandler = reactOnClick(button(t("storageCleanupMoveToSafeTrash")));
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(5);
    expect(previewCleanupOperations).toHaveBeenCalledOnce();
    expect(container.querySelector("[data-workflow-steps]")).not.toBeNull();
    expect(container.querySelector("[data-cleanup-summary]")).not.toBeNull();
    const workflowStates = [...container.querySelectorAll<HTMLElement>("[data-workflow-steps] [data-workflow-state]")].map((step) => step.dataset.workflowState);
    expect(workflowStates).toEqual(expect.arrayContaining(["done", "current", "pending"]));
    expect(container.querySelector('[data-workflow-state="done"]')?.getAttribute("aria-label")).toContain(t("workflowStateDone"));
    expect(container.querySelector('[data-workflow-state="current"]')?.getAttribute("aria-label")).toContain(t("workflowStateCurrent"));
    expect(container.querySelector('[data-workflow-state="pending"]')?.getAttribute("aria-label")).toContain(t("workflowStatePending"));
    expect(container.querySelectorAll("[data-workflow-marker]")).toHaveLength(4);

    await act(async () => button(t("storageCleanupPreviewConfirm")).click());
    await flush(3);
    const confirmMove = [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find((item) => item.textContent?.includes(t("storageCleanupMoveToSafeTrash")));
    expect(confirmMove).toBeDefined();
    const staleMoveHandler = reactOnClick(confirmMove!);
    await act(async () => confirmMove?.click());
    await vi.waitFor(() => expect(moveCleanupCandidatesToSafeTrash).toHaveBeenCalledOnce());
    expect(button("需人工判断").disabled).toBe(true);
    expect(button("可安全清理").disabled).toBe(true);
    expect(findingButton(finding.id, t("storageCleanupSelected")).disabled).toBe(true);
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();

    await act(async () => stalePreviewHandler?.());
    await act(async () => staleMoveHandler?.());
    expect(previewCleanupOperations).toHaveBeenCalledOnce();
    expect(moveCleanupCandidatesToSafeTrash).toHaveBeenCalledOnce();

    resolveMove({ moved: 1, skipped: 0, failed: 0 });
    await flush(10);
    expect(container.textContent).toContain(t("storageCleanupExecutionDone"));
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(button("需人工判断").disabled).toBe(false);
  });

  it("keeps blocked cleanup previews out of confirmation and Safe Trash mutation", async () => {
    const run = makeRun("run-safe-trash-blocked-preview", "completed", 1);
    const finding = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    const moveCleanupCandidatesToSafeTrash = vi.fn(async () => ({ moved: 1, skipped: 0, failed: 0 }));
    const previewCleanupOperations = vi.fn(async () => ({ total: 1, previews: [{
      id: "preview-blocked-safe-trash",
      fileId: finding.id,
      operation_type: "move_to_trash" as const,
      source_path: finding.pathSnapshot ?? "C:/Root/item-0",
      target_path: "Recycle Bin",
      old_name: "item-0",
      new_name: "item-0",
      status: "pending" as const,
      risk_level: "Normal" as const,
      confidence: 1,
      requires_confirmation: true,
      reason: "blocked",
      is_executable: false,
      blocking_reason: "system_trash_source_binding_unsupported"
    }], truncated: false, hasMore: false }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({ findings: request.tier === "review" ? [finding] : [], nextCursor: null, limit: 100 }),
      getAnalysisRun: async () => run,
      previewCleanupOperations,
      moveCleanupCandidatesToSafeTrash
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(finding.id, t("storageCleanupSelectForTrash")).click());
    await flush(3);
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(5);

    expect(previewCleanupOperations).toHaveBeenCalledOnce();
    expect(container.querySelector('[data-workflow-steps] [data-workflow-state="blocked"]')).not.toBeNull();
    expect(container.querySelector('[data-workflow-state="blocked"]')?.getAttribute("aria-label")).toContain(t("workflowStateBlocked"));
    expect(container.querySelector('[data-workflow-marker="blocked"] svg')?.getAttribute("aria-hidden")).toBe("true");
    expect(button(t("storageCleanupPreviewConfirm")).disabled).toBe(true);
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(moveCleanupCandidatesToSafeTrash).not.toHaveBeenCalled();
  });

  it("surfaces authoritative preview failure and keeps Safe Trash unavailable", async () => {
    const run = makeRun("run-safe-trash-preview-failure", "completed", 1);
    const finding = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    const previewCleanupOperations = vi.fn(async () => { throw new Error("preview_failed"); });
    const moveCleanupCandidatesToSafeTrash = vi.fn(async () => ({ moved: 1, skipped: 0, failed: 0 }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({ findings: request.tier === "review" ? [finding] : [], nextCursor: null, limit: 100 }),
      getAnalysisRun: async () => run,
      previewCleanupOperations,
      moveCleanupCandidatesToSafeTrash
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(finding.id, t("storageCleanupSelectForTrash")).click());
    await flush(3);
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(6);

    expect(previewCleanupOperations).toHaveBeenCalledOnce();
    expect(container.textContent).toContain(t("storageCleanupPreviewUnavailableTitle"));
    expect(container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(moveCleanupCandidatesToSafeTrash).not.toHaveBeenCalled();
  });

  it("keeps mixed or incomplete cleanup previews out of Safe Trash mutation", async () => {
    const run = makeRun("run-safe-trash-mixed-preview", "completed", 2);
    const first = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    const second = { ...makeFinding(run, 1), decision: "acknowledged" as const, decisionRevision: 1 };
    const moveCleanupCandidatesToSafeTrash = vi.fn(async () => ({ moved: 2, skipped: 0, failed: 0 }));
    const previewCleanupOperations = vi.fn(async () => ({ total: 2, previews: [{
      id: "preview-mixed-executable",
      fileId: first.id,
      operation_type: "move_to_trash" as const,
      source_path: first.pathSnapshot ?? "C:/Root/item-0",
      target_path: "Recycle Bin",
      old_name: "item-0",
      new_name: "item-0",
      status: "pending" as const,
      risk_level: "Normal" as const,
      confidence: 1,
      requires_confirmation: true,
      reason: "approved",
      is_executable: true
    }, {
      id: "preview-mixed-undefined",
      fileId: second.id,
      operation_type: "move_to_trash" as const,
      source_path: second.pathSnapshot ?? "C:/Root/item-1",
      target_path: "Recycle Bin",
      old_name: "item-1",
      new_name: "item-1",
      status: "pending" as const,
      risk_level: "Normal" as const,
      confidence: 1,
      requires_confirmation: true,
      reason: "not explicitly approved"
    }], truncated: false, hasMore: false }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({ findings: request.tier === "review" ? [first, second] : [], nextCursor: null, limit: 100 }),
      getAnalysisRun: async () => run,
      previewCleanupOperations,
      moveCleanupCandidatesToSafeTrash
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(first.id, t("storageCleanupSelectForTrash")).click());
    await act(async () => findingButton(second.id, t("storageCleanupSelectForTrash")).click());
    await flush(3);
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(5);

    expect(previewCleanupOperations).toHaveBeenCalledOnce();
    expect(button(t("storageCleanupPreviewConfirm")).disabled).toBe(true);
    expect(moveCleanupCandidatesToSafeTrash).not.toHaveBeenCalled();
  });

  it("removes a selected Review finding after AI returns an authoritative Caution revision and clears preview", async () => {
    const run = makeRun("run-ai-reconcile-caution", "completed", 1);
    const selected = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    const caution = { ...selected, tier: "caution", category: "caution", executable: false, decision: null, decisionRevision: null, revision: 2 };
    let aiUpdated = false;
    const listAnalysisFindings = vi.fn(async (request: { tier?: string }) => ({
      findings: request.tier === "review" && !aiUpdated ? [selected] : [],
      nextCursor: null,
      limit: 100
    }));
    const previewCleanupOperations = vi.fn(async () => ({ total: 1, previews: [], truncated: false, hasMore: false }));
    const analyzeCleanupCandidatesWithAI = vi.fn(async () => {
      aiUpdated = true;
      return [{ id: selected.id }] as any;
    });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => aiUpdated ? { ...run, reviewCount: 0, cautionCount: 1 } : run,
      listAnalysisFindings,
      getAnalysisFinding: async () => aiUpdated ? caution : selected,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI,
      previewCleanupOperations
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(6);
    await act(async () => button(t("storageCleanupSelectForTrash")).click());
    await flush(3);
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();

    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(4);
    expect(container.textContent).toContain(t("storageCleanupPreviewReadyTitle"));

    await act(async () => button("重新核验需确认项目").click());
    await flush(10);

    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).not.toContain(t("storageCleanupPreviewReadyTitle"));
    expect(previewCleanupOperations).toHaveBeenCalledOnce();
  });

  it("fails closed when a successful AI mutation cannot be read back", async () => {
    const run = makeRun("run-ai-readback-failure", "completed", 1);
    const selected = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    const previewCleanupOperations = vi.fn(async () => ({ total: 1, previews: [], truncated: false, hasMore: false }));
    const analyzeCleanupCandidatesWithAI = vi.fn(async () => [{ id: selected.id }] as any);
    const listAnalysisFindings = vi.fn(async (request: { tier?: string }) => ({
      findings: request.tier === "review" ? [selected] : [],
      nextCursor: null,
      limit: 100
    }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI,
      getAnalysisFinding: vi.fn(async () => { throw new Error("readback_failed"); }),
      previewCleanupOperations,
      getAnalysisRun: async () => ({ ...run, reviewCount: 1 })
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(selected.id, t("storageCleanupSelectForTrash")).click());
    await flush(2);
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(4);
    expect(container.textContent).toContain(t("storageCleanupPreviewReadyTitle"));
    await act(async () => button("重新核验需确认项目").click());
    await flush(10);

    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).not.toContain(t("storageCleanupPreviewReadyTitle"));
    expect(container.textContent).toContain("已处理 1");
    expect(container.textContent).toContain("权威读回失败 1");
    expect(container.textContent).not.toContain("AI 更新失败 1");
    expect(previewCleanupOperations).toHaveBeenCalledOnce();
  });

  it("keeps Safe selection while removing only the failed Review readback", async () => {
    const run = makeRun("run-ai-readback-partial", "completed", 2, { safeCount: 1 });
    const safe = makeSafeFinding(run, 0);
    const reviewA = { ...makeFinding(run, 1), decision: "acknowledged" as const, decisionRevision: 1 };
    const reviewB = { ...makeFinding(run, 2), decision: "acknowledged" as const, decisionRevision: 1 };
    const refreshedA = { ...reviewA, revision: 2, decisionRevision: 2, updatedAt: 2 };
    const previewCleanupOperations = vi.fn(async (_runId: string, selections: Array<{ findingId: string }>) => {
      expect(new Set(selections.map((selection) => selection.findingId))).toEqual(new Set([safe.id, reviewA.id]));
      return { total: 2, previews: [], truncated: false, hasMore: false };
    });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "safe" ? [safe] : request.tier === "review" ? [reviewA, reviewB] : [],
        nextCursor: null,
        limit: 100
      }),
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI: async () => [{ id: reviewA.id }, { id: reviewB.id }] as any,
      getAnalysisFinding: vi.fn(async (id: string) => id === reviewA.id ? refreshedA : (() => { throw new Error("partial_readback"); })()),
      previewCleanupOperations,
      getAnalysisRun: async () => run
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(reviewA.id, t("storageCleanupSelectForTrash")).click());
    await act(async () => findingButton(reviewB.id, t("storageCleanupSelectForTrash")).click());
    await flush(2);
    await act(async () => button("重新核验需确认项目").click());
    await flush(10);

    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(4);
    expect(previewCleanupOperations).toHaveBeenCalledOnce();
  });

  it("removes uncertain selections conservatively when the AI mutation fails", async () => {
    const run = makeRun("run-ai-mutation-failure", "completed", 1);
    const selected = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    const analyzeCleanupCandidatesWithAI = vi.fn(async () => { throw new Error("ai_failed_after_partial_persist"); });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({ findings: request.tier === "review" ? [selected] : [], nextCursor: null, limit: 100 }),
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI,
      getAnalysisFinding: async () => selected,
      getAnalysisRun: async () => run
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(selected.id, t("storageCleanupSelectForTrash")).click());
    await flush(2);
    await act(async () => button("重新核验需确认项目").click());
    await flush(10);

    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).toContain("AI 更新失败 1");
  });

  it("does not let a pending AI recheck change the active tier", async () => {
    const run = makeRun("run-ai-tier-race", "completed", 1, { safeCount: 1 });
    const review = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    const safe = makeSafeFinding(run, 1);
    let resolveAI: (value: any[]) => void = () => undefined;
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({ findings: request.tier === "review" ? [review] : request.tier === "safe" ? [safe] : [], nextCursor: null, limit: 100 }),
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI: vi.fn(() => new Promise<any[]>((resolve) => { resolveAI = resolve; })),
      getAnalysisFinding: async () => review,
      getAnalysisRun: async () => run
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => button("重新核验需确认项目").click());
    await flush(5);
    await act(async () => button("可安全清理").click());
    await flush(5);
    expect(button("需人工判断").getAttribute("aria-pressed")).toBe("true");
    expect(button("可安全清理").disabled).toBe(true);
    resolveAI([{ id: review.id }]);
    await flush(10);

    expect(container.querySelector('[data-analysis-finding-id="finding-0"]')).not.toBeNull();
    expect(container.querySelector('[data-analysis-finding-id="finding-1"]')).toBeNull();
  });

  it("does not let a pending Review acknowledgement change the active tier", async () => {
    const run = makeRun("run-ack-tier-race", "completed", 1);
    const review = makeFinding(run, 0);
    const caution = { ...review, id: "finding-caution", tier: "caution" as const, category: "caution" as const, executable: false, requiresConfirmation: false, revision: 2 };
    let resolveDecision: (value: { decision: "acknowledged" }) => void = () => undefined;
    const setAnalysisFindingDecision = vi.fn(() => new Promise<{ decision: "acknowledged" }>((resolve) => { resolveDecision = resolve; }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "review" ? [review] : request.tier === "caution" ? [caution] : [],
        nextCursor: null,
        limit: 100
      }),
      getAnalysisFinding: async () => review,
      setAnalysisFindingDecision,
      getAnalysisRun: async () => run
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(review.id, t("storageCleanupFindingAcknowledge")).click());
    await flush(3);
    await act(async () => button(t("storageCleanupReviewConfirmAction")).click());
    await flush(3);
    expect(setAnalysisFindingDecision).toHaveBeenCalledOnce();

    await act(async () => button("谨慎处理").click());
    await flush(5);
    expect(button("需人工判断").getAttribute("aria-pressed")).toBe("true");
    expect(button("谨慎处理").disabled).toBe(true);
    resolveDecision({ decision: "acknowledged" });
    await flush(8);

    expect(container.querySelector('[data-analysis-finding-id="finding-0"]')).not.toBeNull();
    expect(container.querySelector('[data-analysis-finding-id="finding-caution"]')).toBeNull();
  });

  it("does not let a pending stale-finding revalidation change the active tier", async () => {
    const run = makeRun("run-revalidate-tier-race", "completed", 1, { safeCount: 1 });
    const staleReview = { ...makeFinding(run, 0), status: "stale" as const };
    const safe = makeSafeFinding(run, 1);
    let resolveRevalidation: (value: AnalysisFinding) => void = () => undefined;
    const revalidateAnalysisFinding = vi.fn(() => new Promise<AnalysisFinding>((resolve) => { resolveRevalidation = resolve; }));
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "review" ? [staleReview] : request.tier === "safe" ? [safe] : [],
        nextCursor: null,
        limit: 100
      }),
      revalidateAnalysisFinding,
      getAnalysisRun: async () => run
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(5);
    await act(async () => findingButton(staleReview.id, t("storageCleanupFindingRecheck")).click());
    await flush(3);
    expect(revalidateAnalysisFinding).toHaveBeenCalledOnce();

    await act(async () => button("可安全清理").click());
    await flush(5);
    expect(button("需人工判断").getAttribute("aria-pressed")).toBe("true");
    expect(button("可安全清理").disabled).toBe(true);
    resolveRevalidation({ ...staleReview, status: "active", revision: 2 });
    await flush(8);

    expect(container.querySelector('[data-analysis-finding-id="finding-0"]')).not.toBeNull();
    expect(container.querySelector('[data-analysis-finding-id="finding-1"]')).toBeNull();
  });

  it("keeps a selected Review finding on a new authoritative revision and uses that revision for Preview", async () => {
    const run = makeRun("run-ai-reconcile-review", "completed", 1);
    const selected = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    const refreshed = { ...selected, revision: 2, decisionRevision: 2, updatedAt: 2 };
    let aiUpdated = false;
    const listAnalysisFindings = vi.fn(async (request: { tier?: string }) => ({
      findings: request.tier === "review" ? [aiUpdated ? refreshed : selected] : [],
      nextCursor: null,
      limit: 100
    }));
    const previewCleanupOperations = vi.fn(async (_runId: string, selections: Array<{ findingId: string; expectedRevision: number; reviewConfirmation?: { decisionRevision: number } }>) => {
      expect(selections).toEqual([{ findingId: selected.id, expectedRevision: 2, reviewConfirmation: { decisionRevision: 2 } }]);
      return { total: 1, previews: [], truncated: false, hasMore: false };
    });
    const analyzeCleanupCandidatesWithAI = vi.fn(async () => {
      aiUpdated = true;
      return [{ id: selected.id }] as any;
    });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      listAnalysisFindings,
      getAnalysisFinding: async () => refreshed,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI,
      previewCleanupOperations
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(6);
    await act(async () => button(t("storageCleanupSelectForTrash")).click());
    await flush(3);
    await act(async () => button("重新核验需确认项目").click());
    await flush(10);

    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();
    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(4);
    expect(previewCleanupOperations).toHaveBeenCalledOnce();
  });

  it("preserves safe and skipped selections while reconciling only returned authoritative findings", () => {
    const run = makeRun("run-ai-reconcile-pure", "completed", 1, { safeCount: 1 });
    const safe = makeSafeFinding(run, 0);
    const review = { ...makeFinding(run, 1), decision: "acknowledged" as const, decisionRevision: 1 };
    const caution = { ...review, tier: "caution", executable: false, revision: 2 };
    expect(reconcileAuthoritativeFindingUpdates(new Set([safe.id, review.id]), [caution])).toEqual(new Set([safe.id]));
    expect(reconcileAuthoritativeFindingUpdates(new Set([safe.id, review.id]), [])).toEqual(new Set([safe.id, review.id]));
  });

  it("keeps the cleanup scope locked while an AI result is pending", async () => {
    const run = makeRun("run-ai-scope-race", "completed", 1, { paths: ["C:/RootA"] });
    const selected = { ...makeFinding(run, 0), decision: "acknowledged" as const, decisionRevision: 1 };
    let resolveAI: (value: any[]) => void = () => undefined;
    const analyzeCleanupCandidatesWithAI = vi.fn(() => new Promise<any[]>((resolve) => { resolveAI = resolve; }));
    const getAnalysisFinding = vi.fn(async () => selected);
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "review" ? [selected] : [],
        nextCursor: null,
        limit: 100
      }),
      getAnalysisFinding,
      getAISettings: async () => ({ enabled: true, cleanupAiEnabled: true }),
      analyzeCleanupCandidatesWithAI
    });
    dialogMocks.open.mockResolvedValue("C:/RootB");

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootA"], api, t })));
    await flush(8);
    await act(async () => button("需人工判断").click());
    await flush(6);
    await act(async () => button("重新核验需确认项目").click());
    await flush(5);
    expect(analyzeCleanupCandidatesWithAI).toHaveBeenCalledOnce();

    await act(async () => button(t("storageCleanupChooseFolder")).click());
    await flush(4);
    expect(dialogMocks.open).not.toHaveBeenCalled();
    resolveAI([{ id: selected.id }]);
    await flush(8);

    expect(getAnalysisFinding).toHaveBeenCalledWith(selected.id);
    expect(container.textContent).toContain("C:/RootA");
    expect(container.querySelector(`[data-analysis-run-id="${run.id}"]`)).not.toBeNull();
  });

  it("clears the old run, selection, preview, and execution surface when a folder scope changes", async () => {
    const runA = makeRun("run-scope-a", "completed", 0, { paths: ["C:/RootA"], safeCount: 1 });
    const safeFinding = { ...makeSafeFinding(runA, 0), exactReclaimableBytes: 0, potentialReclaimableBytes: 5, sizeBytes: 1 };
    const listAnalysisRuns = vi.fn(async () => [runA]);
    const startAnalysisRun = vi.fn(async () => runA);
    const moveCleanupCandidatesToSafeTrash = vi.fn(async () => ({ moved: 1, skipped: 0, failed: 0 }));
    const api = commonApi(runA, {
      listAnalysisRuns,
      getAnalysisRun: vi.fn(async () => runA),
      listAnalysisFindings: vi.fn(async (request: { tier?: string }) => ({
        findings: request.tier === "safe" ? [safeFinding] : [],
        nextCursor: null,
        limit: 100
      })),
      startAnalysisRun,
      moveCleanupCandidatesToSafeTrash,
      previewCleanupOperations: vi.fn(async () => ({ total: 1, previews: [], truncated: false, hasMore: false }))
    });
    dialogMocks.open.mockResolvedValue("C:/RootB");

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootA"], api, t })));
    await flush(6);
    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).not.toBeNull();
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();
    expect(container.textContent).toContain("5 B");

    await act(async () => button(t("storageCleanupMoveToSafeTrash")).click());
    await flush(4);
    expect(container.textContent).toContain(t("storageCleanupPreviewReadyTitle"));

    await act(async () => button(t("storageCleanupChooseFolder")).click());
    await flush(4);

    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).toBeNull();
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).not.toContain(t("storageCleanupPreviewReadyTitle"));
    expect(container.textContent).toContain("C:/RootB");
    expect(startAnalysisRun).not.toHaveBeenCalled();
    expect(moveCleanupCandidatesToSafeTrash).not.toHaveBeenCalled();
    expect(await listAnalysisRuns.mock.results[0]?.value).toEqual([runA]);
  });

  it("clears the old run when a quick scope is selected without starting a scan", async () => {
    const runA = makeRun("run-quick-a", "completed", 0, { paths: ["C:/RootA"], safeCount: 1 });
    const safeFinding = makeSafeFinding(runA, 0);
    const startAnalysisRun = vi.fn(async () => runA);
    const api = commonApi(runA, {
      listAnalysisRuns: vi.fn(async () => [runA]),
      getAnalysisRun: vi.fn(async () => runA),
      listAnalysisFindings: vi.fn(async (request: { tier?: string }) => ({
        findings: request.tier === "safe" ? [safeFinding] : [],
        nextCursor: null,
        limit: 100
      })),
      startAnalysisRun
    });
    pathMocks.downloadDir.mockResolvedValue("C:/Downloads");

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootA"], api, t })));
    await flush(6);
    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).not.toBeNull();
    expect(container.querySelector("[data-cleanup-selection-summary]")).not.toBeNull();

    await act(async () => button(t("storageCleanupQuickDownloads")).click());
    await flush(4);

    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).toBeNull();
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).toContain("C:/Downloads");
    expect(startAnalysisRun).not.toHaveBeenCalled();
  });

  it("uses a fixed scope only for an explicit PM-01 browser presentation fixture", async () => {
    window.history.replaceState({}, "", "/?pm01-cleanup=preview");
    const run = makeRun("run-presentation-scope", "completed", 0, { paths: ["C:/Users/Zen/Downloads"] });
    const startAnalysisRun = vi.fn(async () => run);
    const api = commonApi(run, { startAnalysisRun });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(6);
    await act(async () => button(t("storageCleanupQuickDownloads")).click());
    await flush(4);

    expect(container.textContent).toContain("C:/Users/Zen/Downloads");
    expect(pathMocks.downloadDir).not.toHaveBeenCalled();
    expect(startAnalysisRun).not.toHaveBeenCalled();
    window.history.replaceState({}, "", "/");
  });

  it("clears the old run when the initialRoots prop changes", async () => {
    const runA = makeRun("run-prop-a", "completed", 0, { paths: ["C:/RootA"], safeCount: 1 });
    const safeFinding = makeSafeFinding(runA, 0);
    const startAnalysisRun = vi.fn(async () => runA);
    const api = commonApi(runA, {
      listAnalysisRuns: vi.fn(async () => [runA]),
      getAnalysisRun: vi.fn(async () => runA),
      listAnalysisFindings: vi.fn(async (request: { tier?: string }) => ({
        findings: request.tier === "safe" ? [safeFinding] : [],
        nextCursor: null,
        limit: 100
      })),
      startAnalysisRun
    });

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootA"], api, t })));
    await flush(6);
    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).not.toBeNull();

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootB"], api, t })));
    await flush(4);

    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).toBeNull();
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).toContain("C:/RootB");
    expect(startAnalysisRun).not.toHaveBeenCalled();
  });

  it("does not hydrate the newest run when it belongs to a different initial scope", async () => {
    const runA = makeRun("run-latest-a", "completed", 0, { paths: ["C:/RootA"], safeCount: 1 });
    const getAnalysisRun = vi.fn(async () => runA);
    const api = commonApi(runA, {
      listAnalysisRuns: async () => [runA],
      getAnalysisRun
    });

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootB"], api, t })));
    await flush(6);

    expect(getAnalysisRun).not.toHaveBeenCalled();
    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).toBeNull();
    expect(container.querySelector("[data-cleanup-selection-summary]")).toBeNull();
    expect(container.textContent).toContain("C:/RootB");
    expect(container.textContent).not.toContain("C:/RootA");
  });

  it("hydrates the older run that exactly matches the requested scope", async () => {
    const runA = makeRun("run-latest-a", "completed", 0, { paths: ["C:/RootA"], safeCount: 1 });
    const runB = makeRun("run-older-b", "completed", 0, { paths: ["c:\\RootB\\"], safeCount: 1 });
    const getAnalysisRun = vi.fn(async (id: string) => id === runB.id ? runB : runA);
    const api = commonApi(runA, {
      listAnalysisRuns: async () => [runA, runB],
      getAnalysisRun,
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "safe" ? [makeSafeFinding(runB, 0)] : [],
        nextCursor: null,
        limit: 100
      })
    });

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootB"], api, t })));
    await flush(8);

    expect(getAnalysisRun).toHaveBeenCalledWith(runB.id);
    expect(getAnalysisRun).not.toHaveBeenCalledWith(runA.id);
    expect(container.querySelector(`[data-analysis-run-id="${runB.id}"]`)).not.toBeNull();
    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).toBeNull();
  });

  it("hydrates the newest legal cleanup run and restores its scope when no scope was selected", async () => {
    const runA = makeRun("run-empty-scope-a", "completed", 0, { paths: ["C:/RootA"], safeCount: 1 });
    const getAnalysisRun = vi.fn(async () => runA);
    const api = commonApi(runA, {
      listAnalysisRuns: async () => [runA],
      getAnalysisRun
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);

    expect(getAnalysisRun).toHaveBeenCalledWith(runA.id);
    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).not.toBeNull();
    expect(container.textContent).toContain("C:/RootA");
  });

  it("does not let a delayed hydration response replace a newly selected scope", async () => {
    const runA = makeRun("run-delayed-a", "completed", 0, { paths: ["C:/RootA"], safeCount: 1 });
    let resolveRuns: (runs: AnalysisRun[]) => void = () => undefined;
    const listAnalysisRuns = vi.fn(() => new Promise<AnalysisRun[]>((resolve) => {
      resolveRuns = resolve;
    }));
    const getAnalysisRun = vi.fn(async () => runA);
    const api = commonApi(runA, { listAnalysisRuns, getAnalysisRun });

    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootA"], api, t })));
    await flush(2);
    await act(async () => root.render(createElement(CleanupView, { initialRoots: ["C:/RootB"], api, t })));
    await flush(2);
    await act(async () => resolveRuns([runA]));
    await flush(6);

    expect(getAnalysisRun).not.toHaveBeenCalled();
    expect(container.querySelector(`[data-analysis-run-id="${runA.id}"]`)).toBeNull();
    expect(container.textContent).toContain("C:/RootB");
    expect(container.textContent).not.toContain("C:/RootA");
  });

  it("hydrates a durable run when initialRoots contain canonical duplicate paths", async () => {
    const run = makeRun("run-canonical-b", "completed", 0, { paths: ["C:/RootB"], safeCount: 1 });
    const getAnalysisRun = vi.fn(async () => run);
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun
    });

    await act(async () => root.render(createElement(CleanupView, {
      initialRoots: ["C:/RootB", "c:\\RootB\\", "C:\\ROOTB"],
      api,
      t
    })));
    await flush(8);

    expect(getAnalysisRun).toHaveBeenCalledWith(run.id);
    expect(container.querySelector(`[data-analysis-run-id="${run.id}"]`)).not.toBeNull();
    expect(container.textContent).toContain("C:/RootB");
  });

  it("matches canonical duplicate roots regardless of order", async () => {
    const run = makeRun("run-canonical-ab", "completed", 0, { paths: ["c:\\rootb\\", "C:/ROOTA"], safeCount: 1 });
    const getAnalysisRun = vi.fn(async () => run);
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun
    });

    await act(async () => root.render(createElement(CleanupView, {
      initialRoots: ["C:/RootA", "C:/RootB", "c:\\roota\\"],
      api,
      t
    })));
    await flush(8);

    expect(getAnalysisRun).toHaveBeenCalledWith(run.id);
    expect(container.querySelector(`[data-analysis-run-id="${run.id}"]`)).not.toBeNull();
  });

  it("does not match a two-root scope with a one-root durable run", async () => {
    const run = makeRun("run-only-a", "completed", 0, { paths: ["C:/RootA"], safeCount: 1 });
    const getAnalysisRun = vi.fn(async () => run);
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun
    });

    await act(async () => root.render(createElement(CleanupView, {
      initialRoots: ["C:/RootA", "C:/RootB"],
      api,
      t
    })));
    await flush(8);

    expect(getAnalysisRun).not.toHaveBeenCalled();
    expect(container.querySelector(`[data-analysis-run-id="${run.id}"]`)).toBeNull();
  });

  it("deduplicates canonical roots in a new scan request while preserving the first display path", async () => {
    const run = makeRun("run-new-scan", "completed", 0, { paths: ["C:/RootA", "C:/RootB"] });
    const requests: Array<{ scope?: { paths?: string[] } }> = [];
    const startAnalysisRun = vi.fn(async (request: { scope?: { paths?: string[] } }) => {
      requests.push(request);
      return run;
    });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [],
      startAnalysisRun,
      getAnalysisRun: vi.fn(async () => run)
    });

    await act(async () => root.render(createElement(CleanupView, {
      initialRoots: ["C:/RootA", "c:\\roota\\", "C:/RootB", "C:\\ROOTB"],
      api,
      t
    })));
    await flush(6);
    await act(async () => scopeButton().click());
    await flush(8);

    expect(startAnalysisRun).toHaveBeenCalledOnce();
    expect(requests[0]?.scope?.paths).toEqual(["C:/RootA", "C:/RootB"]);
  });

  it("remeasures an expanded virtual finding row before positioning the following row", async () => {
    const run = makeRun("run-virtual-measure", "completed", 0, { safeCount: 2 });
    const nativeOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
      configurable: true,
      get() {
        const findingId = this.getAttribute("data-analysis-finding-id");
        if (findingId === "finding-0" && this.querySelector("[data-finding-evidence]")) return 420;
        if (findingId) return 238;
        return 600;
      }
    });
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      getAnalysisRun: async () => run,
      listAnalysisFindings: async (request: { tier?: string }) => ({
        findings: request.tier === "safe" ? [makeSafeFinding(run, 0), makeSafeFinding(run, 1)] : [],
        nextCursor: null,
        limit: 100
      }),
      listAnalysisFindingEvidence: async () => [{ id: "evidence-0", findingId: "finding-0", evidenceKind: "path", subjectKind: "file", subjectId: null, pathSnapshot: "C:/Root/item-0", value: {}, createdAt: 1 }]
    });

    try {
      await act(async () => root.render(createElement(CleanupView, { api, t })));
      await flush(8);
      const virtualList = container.querySelector<HTMLElement>("[data-cleanup-findings] .relative");
      const initialTotalSize = Number.parseFloat(virtualList?.style.height ?? "0");
      const secondRow = container.querySelector<HTMLElement>('[data-analysis-finding-id="finding-1"]');
      expect(secondRow?.style.transform).toBe("translateY(238px)");

      const evidenceButton = [...(container.querySelector('[data-analysis-finding-id="finding-0"]')?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((button) => button.textContent?.includes("查看证据"));
      expect(evidenceButton).toBeDefined();
      await act(async () => evidenceButton?.click());
      await flush(8);

      expect(container.querySelector('[data-analysis-finding-id="finding-0"] [data-finding-evidence]')).not.toBeNull();
      expect(Number.parseFloat(virtualList?.style.height ?? "0")).toBeGreaterThan(initialTotalSize);
      expect(container.querySelector<HTMLElement>('[data-analysis-finding-id="finding-1"]')?.style.transform).toBe("translateY(420px)");
    } finally {
      if (nativeOffsetHeight) Object.defineProperty(HTMLElement.prototype, "offsetHeight", nativeOffsetHeight);
      else delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
    }
  });

  it("keeps two concurrently loaded evidence panels expanded", async () => {
    const run = makeRun("run-evidence-race", "completed", 0, { safeCount: 2 });
    const first = makeSafeFinding(run, 0);
    const second = makeSafeFinding(run, 1);
    let resolveFirst: (value: any[]) => void = () => undefined;
    let resolveSecond: (value: any[]) => void = () => undefined;
    const evidence = (findingId: string) => [{ id: `evidence-${findingId}`, findingId, evidenceKind: "path", subjectKind: "file", subjectId: null, pathSnapshot: `C:/Root/${findingId}`, value: {}, createdAt: 1 }];
    const api = commonApi(run, {
      listAnalysisRuns: async () => [run],
      listAnalysisFindings: async (request: { tier?: string }) => ({ findings: request.tier === "safe" ? [first, second] : [], nextCursor: null, limit: 100 }),
      listAnalysisFindingEvidence: vi.fn((findingId: string) => findingId === first.id
        ? new Promise<any[]>((resolve) => { resolveFirst = resolve; })
        : new Promise<any[]>((resolve) => { resolveSecond = resolve; }))
    });

    await act(async () => root.render(createElement(CleanupView, { api, t })));
    await flush(8);
    await act(async () => findingButton(first.id, "查看证据").click());
    await act(async () => findingButton(second.id, "查看证据").click());
    resolveSecond(evidence(second.id));
    resolveFirst(evidence(first.id));
    await flush(8);

    expect(container.querySelector(`[data-analysis-finding-id="${first.id}"] [data-finding-evidence]`)).not.toBeNull();
    expect(container.querySelector(`[data-analysis-finding-id="${second.id}"] [data-finding-evidence]`)).not.toBeNull();
  });

  it("matches Windows extended-length drive and UNC paths", async () => {
    const driveRun = makeRun("run-extended-drive", "completed", 0, { paths: ["C:/Root"], safeCount: 1 });
    const uncRun = makeRun("run-extended-unc", "completed", 0, { paths: ["\\\\server\\share"], safeCount: 1 });
    const getAnalysisRun = vi.fn(async (id: string) => id === driveRun.id ? driveRun : uncRun);
    const driveApi = commonApi(driveRun, {
      listAnalysisRuns: async () => [driveRun],
      getAnalysisRun
    });

    await act(async () => root.render(createElement(CleanupView, {
      initialRoots: ["\\\\?\\C:\\Root"],
      api: driveApi,
      t
    })));
    await flush(8);
    expect(getAnalysisRun).toHaveBeenCalledWith(driveRun.id);

    act(() => root.unmount());
    document.body.innerHTML = '<div id="test-root"></div>';
    container = document.getElementById("test-root") as HTMLDivElement;
    root = createRoot(container);
    const uncApi = commonApi(uncRun, {
      listAnalysisRuns: async () => [uncRun],
      getAnalysisRun
    });
    await act(async () => root.render(createElement(CleanupView, {
      initialRoots: ["\\\\?\\UNC\\server\\share"],
      api: uncApi,
      t
    })));
    await flush(8);

    expect(getAnalysisRun).toHaveBeenCalledWith(uncRun.id);
  });
});
