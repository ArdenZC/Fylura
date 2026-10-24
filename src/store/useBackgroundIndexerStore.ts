import { create } from "zustand";
import { makeTranslator } from "../i18n";
import { isScanAdmissionBusy } from "../utils/scanAdmission";
import { tauriApi, type ManagedScanRequest, type ManagedScanStartDto, type ScanSessionDto } from "../api/tauriApi";
import { normalizePathLike, readableError } from "../utils/viewHelpers";
import { useAppStore } from "./useAppStore";
import { reportBackgroundIndexerCancelFailure } from "./backgroundIndexerErrors";
import { useFileLibraryStore } from "./useFileLibraryStore";
import { useScanManagerStore } from "./useScanManagerStore";

export interface BackgroundIndexFailure {
  path: string;
  message: string;
}

export interface BackgroundIndexEnqueueOptions {
  force?: boolean;
}

export interface BackgroundIndexerStore {
  pendingRoots: string[];
  currentRoot: string | null;
  isBackgroundIndexing: boolean;
  failedRoots: BackgroundIndexFailure[];
  completedRoots: string[];
  enqueueRoot: (path: string, options?: BackgroundIndexEnqueueOptions) => void;
  enqueueRoots: (paths: string[], options?: BackgroundIndexEnqueueOptions) => void;
  cancelBackgroundIndexing: () => Promise<void>;
}

const BACKGROUND_INDEXER_FOREGROUND_WAIT_MS = 1200;
const MAX_BACKGROUND_INDEX_HISTORY = 6;
const MAX_RECENTLY_INDEXED_ROOTS = 200;

let isProcessingBackgroundQueue = false;
let activeBackgroundJobId: string | null = null;
let activeBackgroundSessionId: string | null = null;
let backgroundGeneration = 0;
let backgroundCancelRequestId = 0;
let backgroundCancelInFlight: Promise<void> | null = null;
const recentlyIndexedRoots = new Set<string>();
const recentlyIndexedRootOrder: string[] = [];

export const useBackgroundIndexerStore = create<BackgroundIndexerStore>((set, get) => ({
  pendingRoots: [],
  currentRoot: null,
  isBackgroundIndexing: false,
  failedRoots: [],
  completedRoots: [],
  enqueueRoot: (path, options) => get().enqueueRoots([path], options),
  enqueueRoots: (paths, options) => {
    const normalizedPaths = uniqueNormalizedRoots(paths);
    if (!normalizedPaths.length) return;

    let queuedRoots = false;
    set((state) => {
      const knownRoots = new Set([
        ...state.pendingRoots.map(normalizeRoot),
        ...(state.currentRoot ? [normalizeRoot(state.currentRoot)] : []),
        ...(!options?.force ? state.completedRoots.map(normalizeRoot) : []),
        ...(!options?.force ? Array.from(recentlyIndexedRoots) : [])
      ]);
      const nextRoots = normalizedPaths.filter((path) => !knownRoots.has(normalizeRoot(path)));
      if (!nextRoots.length) return state;
      queuedRoots = true;
      return { pendingRoots: [...state.pendingRoots, ...nextRoots] };
    });

    if (queuedRoots) scheduleBackgroundIndexing();
  },
  cancelBackgroundIndexing: async () => {
    if (backgroundCancelInFlight) return backgroundCancelInFlight;

    const jobId = activeBackgroundJobId;
    if (!jobId) {
      // No job id means the queue is between roots or still waiting for the
      // backend job id. Invalidate that run locally so it cannot start after
      // the queue has been cleared.
      backgroundGeneration += 1;
      set({ pendingRoots: [], currentRoot: null, isBackgroundIndexing: false });
      return;
    }
    const requestId = ++backgroundCancelRequestId;
    const runGeneration = backgroundGeneration;
    let request!: Promise<void>;
    request = (async () => {
      try {
        await tauriApi.cancelScanRun(jobId);
        if (
          requestId !== backgroundCancelRequestId
          || runGeneration !== backgroundGeneration
          || activeBackgroundJobId !== jobId
        ) return;

        // Only a confirmed backend cancellation invalidates the run. Until
        // this point a late successful startScan result remains authoritative.
        backgroundGeneration += 1;
        activeBackgroundJobId = null;
        activeBackgroundSessionId = null;
        set({ pendingRoots: [], currentRoot: null, isBackgroundIndexing: false });
      } catch (error) {
        // Keep the active generation and job intact so a late startScan result
        // is still accepted when the cancellation RPC fails.
        if (requestId === backgroundCancelRequestId) {
          reportBackgroundIndexerCancelFailure(error);
        }
      } finally {
        if (backgroundCancelInFlight === request) backgroundCancelInFlight = null;
      }
    })();
    backgroundCancelInFlight = request;
    return request;
  }
}));

function scheduleBackgroundIndexing() {
  if (isProcessingBackgroundQueue) return;
  isProcessingBackgroundQueue = true;
  void processBackgroundQueue().finally(() => {
    isProcessingBackgroundQueue = false;
    if (useBackgroundIndexerStore.getState().pendingRoots.length > 0) {
      scheduleBackgroundIndexing();
    }
  });
}

async function processBackgroundQueue() {
  const runGeneration = backgroundGeneration;
  while (true) {
    if (runGeneration !== backgroundGeneration) return;

    if (useScanManagerStore.getState().isScanning) {
      useBackgroundIndexerStore.setState({ currentRoot: null, isBackgroundIndexing: false });
      await delay(BACKGROUND_INDEXER_FOREGROUND_WAIT_MS);
      continue;
    }

    const root = useBackgroundIndexerStore.getState().pendingRoots[0];
    if (!root) {
      useBackgroundIndexerStore.setState({ currentRoot: null, isBackgroundIndexing: false });
      return;
    }

    useBackgroundIndexerStore.setState((state) => ({
      pendingRoots: state.pendingRoots.slice(1),
      currentRoot: root,
      isBackgroundIndexing: true
    }));

    let jobId: string | null = null;
    try {
      const request: ManagedScanRequest = {
        roots: [root],
        requestKey: `background-scan-${globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`}`,
        dedupe: true
      };
      const start: ManagedScanStartDto = await tauriApi.startManagedScan(request);
      activeBackgroundSessionId = start.session.id;
      jobId = start.runs[0]?.id ?? start.session.id;
      if (runGeneration !== backgroundGeneration) return;
      activeBackgroundJobId = jobId;
      const session = await waitForManagedBackgroundSession(request, start);
      if (session.status === "cancelled") return;
      // `requires_reconciliation` is an index-health signal, not a failed job: the run
      // finished and persisted what it observed. Treating it as a failure stopped the
      // root from being recorded as indexed and made the queue retry it forever.
      if (!["completed", "completed_with_warnings", "requires_reconciliation"].includes(session.status)) {
        throw new Error(session.errorMessage ?? `Background scan ended in ${session.status}.`);
      }
      if (runGeneration === backgroundGeneration && activeBackgroundJobId === jobId) {
        markRecentlyIndexedRoot(root);
        useBackgroundIndexerStore.setState((state) => ({
          completedRoots: [root, ...state.completedRoots.filter((item) => normalizeRoot(item) !== normalizeRoot(root))]
            .slice(0, MAX_BACKGROUND_INDEX_HISTORY),
          failedRoots: state.failedRoots.filter((item) => normalizeRoot(item.path) !== normalizeRoot(root))
        }));
        await useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery);
      }
    } catch (error) {
      if (runGeneration === backgroundGeneration && activeBackgroundJobId === jobId) {
        if (jobId === null && isScanAdmissionBusy(error)) {
          // Another owner retains the lease. Do not fail or cache this root as
          // indexed; a subsequent enqueue remains eligible after it finishes.
          useBackgroundIndexerStore.setState((state) => ({
            failedRoots: state.failedRoots.filter((item) => normalizeRoot(item.path) !== normalizeRoot(root))
          }));
          useAppStore.getState().showToast({
            type: "info",
            message: makeTranslator(useAppStore.getState().language)("scanAlreadyInProgress")
          });
          continue;
        }
        const message = readableError(error);
        useBackgroundIndexerStore.setState((state) => ({
          failedRoots: [
            { path: root, message },
            ...state.failedRoots.filter((item) => normalizeRoot(item.path) !== normalizeRoot(root))
          ].slice(0, MAX_BACKGROUND_INDEX_HISTORY)
        }));
      }
    } finally {
      const ownsRun = runGeneration === backgroundGeneration && activeBackgroundJobId === jobId;
      if (activeBackgroundJobId === jobId) activeBackgroundJobId = null;
      if (ownsRun) activeBackgroundSessionId = null;
      if (ownsRun) {
        useBackgroundIndexerStore.setState((state) => ({
          currentRoot: state.currentRoot === root ? null : state.currentRoot,
          isBackgroundIndexing: false
        }));
      }
    }
  }
}

async function waitForManagedBackgroundSession(
  request: ManagedScanRequest,
  initial: ManagedScanStartDto
): Promise<ScanSessionDto> {
  let current = initial.session;
  while (!["cancelled", "completed", "completed_with_warnings", "failed", "interrupted", "requires_reconciliation"].includes(current.status)) {
    await delay(250);
    const snapshot = await tauriApi.startManagedScan(request);
    current = snapshot.session;
  }
  return current;
}

function delay(ms: number) {
  return new Promise((resolve) => globalThis.setTimeout(resolve, ms));
}

function uniqueNormalizedRoots(paths: string[]) {
  const seen = new Set<string>();
  const roots: string[] = [];
  for (const path of paths) {
    const normalized = normalizeRoot(path);
    if (!normalized || seen.has(normalized)) continue;
    seen.add(normalized);
    roots.push(path.trim());
  }
  return roots;
}

function markRecentlyIndexedRoot(path: string) {
  const normalized = normalizeRoot(path);
  if (!normalized) return;
  if (!recentlyIndexedRoots.has(normalized)) {
    recentlyIndexedRootOrder.push(normalized);
  }
  recentlyIndexedRoots.add(normalized);
  while (recentlyIndexedRootOrder.length > MAX_RECENTLY_INDEXED_ROOTS) {
    const staleRoot = recentlyIndexedRootOrder.shift();
    if (staleRoot) recentlyIndexedRoots.delete(staleRoot);
  }
}

function normalizeRoot(path: string) {
  return normalizePathLike(path.trim());
}
