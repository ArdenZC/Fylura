import { beforeEach, describe, expect, it, vi } from "vitest";

const runtimeMocks = vi.hoisted(() => ({
  startManagedScan: vi.fn(),
  cancelScanRun: vi.fn(),
  refresh: vi.fn(),
  showError: vi.fn(),
  showToast: vi.fn(),
  isScanning: false
}));

vi.mock("../src/api/tauriApi", () => ({
  tauriApi: {
    startManagedScan: runtimeMocks.startManagedScan,
    cancelScanRun: runtimeMocks.cancelScanRun
  }
}));

vi.mock("../src/store/useAppStore", () => ({
  useAppStore: { getState: () => ({ searchQuery: "", showError: runtimeMocks.showError, showToast: runtimeMocks.showToast, language: "zh" }) }
}));

vi.mock("../src/store/useFileLibraryStore", () => ({
  useFileLibraryStore: { getState: () => ({ refresh: runtimeMocks.refresh }) }
}));

vi.mock("../src/store/useScanManagerStore", () => ({
  useScanManagerStore: { getState: () => ({ isScanning: runtimeMocks.isScanning }) }
}));

import { useBackgroundIndexerStore } from "../src/store/useBackgroundIndexerStore";

describe("background indexer lifecycle", () => {
  beforeEach(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
    runtimeMocks.startManagedScan.mockReset();
    runtimeMocks.cancelScanRun.mockReset().mockResolvedValue({});
    runtimeMocks.refresh.mockReset().mockResolvedValue(undefined);
    runtimeMocks.showError.mockReset();
    runtimeMocks.showToast.mockReset();
    runtimeMocks.isScanning = false;
    useBackgroundIndexerStore.setState({
      pendingRoots: [],
      currentRoot: null,
      isBackgroundIndexing: false,
      failedRoots: [],
      completedRoots: []
    });
  });

  it.each([
    "Scan root already has an active run: F:/busy-m1",
    "Scan root overlaps an active root lease: F:/busy-m1 overlaps F:/busy-m1/child (run-1)"
  ])("reports occupied admission as nonfatal without completing or caching the root: %s", async (message) => {
    const root = `F:/busy-m1-${message.includes("overlaps") ? "overlap" : "same"}`;
    runtimeMocks.startManagedScan.mockRejectedValueOnce(new Error(`Validation error: ${message}`));
    useBackgroundIndexerStore.getState().enqueueRoot(root);
    await flushPromises();
    expect(useBackgroundIndexerStore.getState().failedRoots).toEqual([]);
    expect(useBackgroundIndexerStore.getState().completedRoots).toEqual([]);
    expect(useBackgroundIndexerStore.getState().isBackgroundIndexing).toBe(false);
    expect(runtimeMocks.showError).not.toHaveBeenCalled();
    expect(runtimeMocks.showToast).toHaveBeenCalledWith(expect.objectContaining({ type: "info" }));
    expect(runtimeMocks.refresh).not.toHaveBeenCalled();

    runtimeMocks.startManagedScan.mockResolvedValue(managedStart("completed", root));
    useBackgroundIndexerStore.getState().enqueueRoot(root);
    await flushPromises();
    expect(runtimeMocks.startManagedScan).toHaveBeenCalledTimes(2);
    expect(useBackgroundIndexerStore.getState().completedRoots).toEqual([root]);
  });

  it("ignores a stale scan completion after cancellation", async () => {
    const start = deferred<void>();
    runtimeMocks.startManagedScan.mockReturnValue(start.promise);

    useBackgroundIndexerStore.getState().enqueueRoot("F:/hardening");
    await flushPromises();
    expect(useBackgroundIndexerStore.getState().isBackgroundIndexing).toBe(true);

    const cancelling = useBackgroundIndexerStore.getState().cancelBackgroundIndexing();
    await cancelling;
    expect(useBackgroundIndexerStore.getState().isBackgroundIndexing).toBe(false);

    start.resolve();
    await flushPromises();

    expect(useBackgroundIndexerStore.getState().completedRoots).toEqual([]);
    expect(runtimeMocks.refresh).not.toHaveBeenCalled();
    expect(useBackgroundIndexerStore.getState().currentRoot).toBeNull();
  });

  it("keeps the active job truthful when cancellation RPC fails", async () => {
    runtimeMocks.startManagedScan
      .mockResolvedValueOnce(managedStart("running"))
      .mockResolvedValue(managedStart("completed"));
    runtimeMocks.cancelScanRun.mockRejectedValue(new Error("cancel unavailable"));

    useBackgroundIndexerStore.getState().enqueueRoot("F:/hardening");
    await flushPromises();
    await new Promise((resolve) => setTimeout(resolve, 60));
    await useBackgroundIndexerStore.getState().cancelBackgroundIndexing();

    expect(useBackgroundIndexerStore.getState().isBackgroundIndexing).toBe(true);
    expect(useBackgroundIndexerStore.getState().currentRoot).toBe("F:/hardening");
    expect(runtimeMocks.showError).toHaveBeenCalledWith(expect.stringContaining("cancel"));

    await new Promise((resolve) => setTimeout(resolve, 300));
    await flushPromises();

    expect(useBackgroundIndexerStore.getState().completedRoots).toEqual(["F:/hardening"]);
    expect(runtimeMocks.refresh).toHaveBeenCalledTimes(1);
  });

  // Acceptance: a run that ends with incomplete coverage is a finished job, not a
  // failed one. Pins behaviour rather than the status name — the root must be recorded
  // as indexed and no error surfaced, whatever the backend calls that terminal state.
  it("records the root as indexed when the scan finished with incomplete coverage", async () => {
    // A distinct root: `recentlyIndexedRoots` is module-level state that outlives
    // `beforeEach`, so reusing an earlier test's path would be silently de-duplicated.
    const root = "F:/incomplete-coverage";
    runtimeMocks.startManagedScan.mockResolvedValue(managedStart("requires_reconciliation", root));

    useBackgroundIndexerStore.getState().enqueueRoot(root);
    await flushPromises();
    await new Promise((resolve) => setTimeout(resolve, 60));
    await flushPromises();

    expect(useBackgroundIndexerStore.getState().completedRoots).toEqual([root]);
    expect(useBackgroundIndexerStore.getState().failedRoots).toEqual([]);
    expect(runtimeMocks.showError).not.toHaveBeenCalled();
    expect(runtimeMocks.refresh).toHaveBeenCalledTimes(1);
  });
});

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function managedStart(
  status: "running" | "completed" | "requires_reconciliation",
  rootPath = "F:/hardening"
) {
  const terminal = status !== "running";
  const run = {
    id: "background-job-1",
    scanRootId: "background-root-1",
    rootPath,
    generation: 1,
    parentSessionId: "background-session-1",
    status,
    phase: terminal ? "completed" : "discovering",
    scannedFiles: terminal ? 2 : 0,
    scannedDirectories: terminal ? 1 : 0,
    processedBytes: terminal ? 2 : 0,
    warningsCount: 0,
    errorsCount: 0,
    metadataErrorCount: 0,
    coverageErrorCount: 0,
    coverageComplete: terminal,
    staleReconciliationAllowed: false,
    cancelRequested: false,
    revision: terminal ? 4 : 1,
    sessionRevision: terminal ? 4 : 1,
    startedAt: 1,
    finishedAt: terminal ? 2 : null,
    lastCheckpointAt: 1,
    errorCode: null,
    errorMessage: null,
    resultJson: null,
    createdAt: 1,
    updatedAt: 1
  };
  return {
    session: {
      id: "background-session-1",
      requestKey: "background-request-1",
      canonicalRequestHash: "hash",
      status,
      phase: terminal ? "completed" : "running",
      cancelRequested: false,
      requestedRootCount: 1,
      effectiveRootCount: 1,
      completedRootCount: terminal ? 1 : 0,
      failedRootCount: 0,
      cancelledRootCount: 0,
      coveredRootCount: 0,
      unstartedRootCount: terminal ? 0 : 1,
      dedupeRequested: true,
      dedupeDispatchState: terminal ? "pending" : "not_requested",
      dedupeAttemptCount: 0,
      dedupeJobId: null,
      dedupeLastError: null,
      scannedFiles: run.scannedFiles,
      scannedDirectories: run.scannedDirectories,
      warningsCount: 0,
      errorsCount: 0,
      revision: terminal ? 4 : 1,
      startedAt: 1,
      finishedAt: terminal ? 2 : null,
      lastCheckpointAt: 1,
      errorCode: null,
      errorMessage: null,
      resultJson: null,
      createdAt: 1,
      updatedAt: 1,
      roots: [{
        sessionId: "background-session-1",
        requestedIndex: 0,
        requestedPath: rootPath,
        normalizedRequestedPath: rootPath,
        resolution: "effective",
        effectiveRootId: "background-root-1",
        effectivePath: rootPath,
        effectiveIndex: 0,
        runId: "background-job-1",
        status,
        reason: null,
        createdAt: 1,
        updatedAt: 1
      }]
    },
    runs: [run]
  };
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}
