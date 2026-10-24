import { create } from "zustand";
import { isScanAdmissionBusy } from "../utils/scanAdmission";
import { open } from "@tauri-apps/plugin-dialog";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  tauriApi,
  type DedupeCompletePayload,
  type DedupeProgressPayload,
  type ManagedScanEvent,
  type ManagedScanRequest,
  type ManagedScanStartDto,
  type ScanBatchPayload,
  type ScanProgressPayload,
  type ScanRunDto,
  type ScanSessionDto,
  type ScanSummary,
  type ScannedEntry
} from "../api/tauriApi";
import { enabledScanRootPaths } from "../hooks/useAppSettings";
import { makeTranslator } from "../i18n";
import type { ScanRootSetting } from "../types/domain";
import { registerListenerGroup } from "../utils/registerListenerGroup";
import { readableError } from "../utils/viewHelpers";
import { useAppStore } from "./useAppStore";
import { useFileLibraryStore } from "./useFileLibraryStore";

export type ScanStatus = "idle" | "scanning" | "completed" | "canceled" | "error";

export interface ScanStateData {
  status: ScanStatus;
  progress: ScanProgressPayload | null;
  entries: ScannedEntry[];
  error: string | null;
}

const initialScanState: ScanStateData = {
  status: "idle",
  progress: null,
  entries: [],
  error: null
};

const terminalRunStatuses = new Set([
  "cancelled",
  "completed",
  "completed_with_warnings",
  "failed",
  "interrupted",
  "requires_reconciliation"
]);

let scanJobCanceled = false;
let activeScanJobId: string | null = null;
let activeManagedSessionId: string | null = null;
let activeManagedRequest: ManagedScanRequest | null = null;
let activeDedupeParentScanJobId: string | null = null;
let activeDedupeJobId: string | null = null;

export function isCurrentDedupeEvent(
  payload: Pick<DedupeProgressPayload, "dedupeJobId" | "parentScanJobId">,
  parentScanJobId: string | null,
  dedupeJobId: string | null
) {
  return payload.parentScanJobId === parentScanJobId
    && (dedupeJobId === null || payload.dedupeJobId === dedupeJobId);
}

export type ManagedEventDecision = "accept" | "ignore" | "refresh";

export function decideManagedScanEvent(
  event: ManagedScanEvent,
  expectedSessionId: string | null,
  knownRunRevision: number | undefined,
  knownRunGeneration: number | undefined,
  knownSessionRevision: number,
  knownRunStatus: string | undefined,
  seenEventIds: readonly string[]
): ManagedEventDecision {
  if (!expectedSessionId || event.parentSessionId !== expectedSessionId) return "ignore";
  if (seenEventIds.includes(event.eventId)) return "ignore";
  if (knownRunRevision === undefined) return "refresh";
  if (knownRunGeneration !== undefined && event.generation !== knownRunGeneration) return "ignore";
  if (knownRunStatus && terminalRunStatuses.has(knownRunStatus) && !terminalRunStatuses.has(event.status)) {
    return "ignore";
  }
  if (knownRunRevision !== undefined && event.runRevision < knownRunRevision) return "ignore";
  if (event.sessionRevision < knownSessionRevision) return "ignore";
  if (knownRunRevision !== undefined && event.runRevision === knownRunRevision) return "refresh";
  if (event.sessionRevision > knownSessionRevision + 1) return "refresh";
  if (knownRunRevision !== undefined && event.runRevision > knownRunRevision + 1) return "refresh";
  return "accept";
}

export interface ScanManagerStore {
  selectedFolders: string[];
  defaultScanRoots: ScanRootSetting[];
  isScanning: boolean;
  isCancelingScan: boolean;
  scanState: ScanStateData;
  activeScanSessionId: string | null;
  activeScanRunId: string | null;
  scanSession: ScanSessionDto | null;
  scanRuns: ScanRunDto[];
  lastRunRevision: number;
  lastSessionRevision: number;
  seenManagedEventIds: string[];
  listenersRegistered: boolean;
  registrationPromise: Promise<void> | null;
  unlisteners: UnlistenFn[];
  initializeScanListeners: () => Promise<void>;
  refreshManagedScanState: () => Promise<void>;
  setDefaultScanRoots: (roots: ScanRootSetting[]) => void;
  reset: () => void;
  scanPath: (path: string) => Promise<void>;
  scanPaths: (paths: string[]) => Promise<void>;
  handleScan: () => Promise<void>;
  handleChooseFolders: () => Promise<void>;
  cancelScan: () => Promise<void>;
}

function currentT() {
  return makeTranslator(useAppStore.getState().language);
}

function nextRequestKey() {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  return `scan-session-${randomUuid ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`}`;
}

function isTerminalSessionStatus(status: string | undefined) {
  return terminalRunStatuses.has(status ?? "");
}

// Maps a backend *job outcome* onto the UI's job state.  These axes are kept separate
// on purpose: whether the scan ran to completion, whether the resulting index is
// healthy, and whether the generation pointer advanced are three independent facts.
// `requires_reconciliation` says the run finished but its coverage was incomplete —
// that is an index-health signal, not a failed job, so it must not be shown as an
// error.  The health signal is preserved separately via `indexIncomplete` below.
export function scanStatusForBackendStatus(status: string): ScanStatus {
  if (status === "completed" || status === "completed_with_warnings") return "completed";
  if (status === "requires_reconciliation") return "completed";
  if (status === "cancelled" || status === "cancelled_not_started") return "canceled";
  if (status === "failed" || status === "interrupted") return "error";
  return "scanning";
}

// Axis 2: the index-health signal that `scanStatusForBackendStatus` deliberately drops.
export function indexIsIncompleteForStatus(status: string | undefined) {
  return status === "requires_reconciliation";
}

function progressFromManagedEvent(event: ManagedScanEvent): ScanProgressPayload {
  return {
    jobId: event.runId,
    jobKind: "foreground",
    root: event.currentPath ?? "",
    scanned: event.scannedFiles + event.scannedDirectories,
    files: event.scannedFiles,
    directories: event.scannedDirectories,
    skipped: 0,
    errors: event.errorsCount,
    elapsedMs: 0
  };
}

function projectRunFromEvent(previous: ScanRunDto | undefined, event: ManagedScanEvent): ScanRunDto {
  return {
    id: event.runId,
    scanRootId: event.scanRootId,
    rootPath: previous?.rootPath ?? event.currentPath ?? "",
    generation: event.generation,
    parentSessionId: event.parentSessionId,
    status: event.status,
    phase: event.runPhase,
    scannedFiles: event.scannedFiles,
    scannedDirectories: event.scannedDirectories,
    processedBytes: event.processedBytes,
    warningsCount: event.warningsCount,
    errorsCount: event.errorsCount,
    metadataErrorCount: previous?.metadataErrorCount ?? 0,
    coverageErrorCount: previous?.coverageErrorCount ?? 0,
    coverageComplete: previous?.coverageComplete ?? false,
    staleReconciliationAllowed: previous?.staleReconciliationAllowed ?? false,
    cancelRequested: previous?.cancelRequested ?? event.status === "cancelling",
    revision: event.runRevision,
    sessionRevision: event.sessionRevision,
    startedAt: previous?.startedAt ?? null,
    finishedAt: previous?.finishedAt ?? (terminalRunStatuses.has(event.status) ? event.timestamp : null),
    lastCheckpointAt: event.timestamp,
    errorCode: event.errorCode,
    errorMessage: event.errorMessage,
    resultJson: previous?.resultJson ?? null,
    watcherRevisionAtStart: previous?.watcherRevisionAtStart ?? 0,
    createdAt: previous?.createdAt ?? event.timestamp,
    updatedAt: event.timestamp
  };
}

function sessionStatusFromMappings(session: ScanSessionDto, runs: ScanRunDto[]): ScanSessionDto {
  const byRunId = new Map(runs.map((run) => [run.id, run]));
  const roots = session.roots.map((root) => {
    const run = root.runId ? byRunId.get(root.runId) : undefined;
    if (!run) return root;
    return { ...root, status: run.status, updatedAt: run.updatedAt };
  });
  const statuses = roots.map((root) => root.status);
  const terminal = statuses.every((status) => terminalRunStatuses.has(status) || ["covered", "duplicate", "nested", "invalid"].includes(status));
  const status = !terminal
    ? session.cancelRequested ? "cancelling" : "running"
    : statuses.includes("requires_reconciliation") ? "requires_reconciliation"
      : statuses.includes("interrupted") ? "interrupted"
        : statuses.some((value) => value === "failed" || value === "invalid") ? "failed"
          : statuses.some((value) => value === "cancelled" || value === "cancelled_not_started") ? "cancelled"
            : statuses.includes("completed_with_warnings") ? "completed_with_warnings"
              : "completed";
  const phase = terminal
    ? "completed"
    : session.phase === "finalizing" || session.phase === "completed"
      ? session.phase
      : "running";
  return { ...session, roots, status, phase };
}

function applyManagedStartSnapshot(start: ManagedScanStartDto) {
  const runRevisions = Object.fromEntries(start.runs.map((run) => [run.id, run.revision]));
  setManagedSessionState({
    session: start.session,
    runs: start.runs,
    runRevisions,
    sessionRevision: start.session.revision,
    sessionId: start.session.id
  });
}

function setManagedSessionState(input: {
  session: ScanSessionDto;
  runs: ScanRunDto[];
  runRevisions: Record<string, number>;
  sessionRevision: number;
  sessionId: string;
}) {
  useScanManagerStore.setState((state) => ({
    activeScanSessionId: input.sessionId,
    activeScanRunId: input.runs.find((run) => ["queued", "running", "cancelling"].includes(run.status))?.id ?? null,
    scanSession: input.session,
    scanRuns: input.runs,
    lastRunRevision: Math.max(0, ...Object.values(input.runRevisions)),
    lastSessionRevision: input.sessionRevision,
    seenManagedEventIds: state.seenManagedEventIds
  }));
}

async function hydrateManagedScanState() {
  const request = activeManagedRequest;
  if (request) {
    const snapshot = await tauriApi.startManagedScan(request);
    applyManagedStartSnapshot(snapshot);
    return;
  }

  const persistedSessionId = activeManagedSessionId ?? persistedScanSessionId();
  if (persistedSessionId) activeManagedSessionId = persistedSessionId;
  const sessionId = persistedSessionId
    ?? (await tauriApi.listScanRuns(undefined, undefined, 100))
      .find((run) => run.parentSessionId && ["queued", "running", "cancelling"].includes(run.status))
      ?.parentSessionId;
  if (!sessionId) return;
  const snapshot = await tauriApi.getManagedScanSnapshot(sessionId);
  activeManagedSessionId = sessionId;
  applyManagedStartSnapshot({
    session: snapshot.session,
    runs: snapshot.runs
  });
  useScanManagerStore.setState((state) => ({
    scanState: {
      ...state.scanState,
      status: scanStatusForBackendStatus(snapshot.session.status),
      progress: state.scanState.progress
    }
  }));
}

function persistedScanSessionId() {
  const scope = useFileLibraryStore.getState().scope;
  return scope.kind === "current_scan" ? scope.scanSessionId ?? null : null;
}

async function waitForManagedSession(sessionId: string) {
  while (activeManagedSessionId === sessionId) {
    const current = useScanManagerStore.getState().scanSession;
    if (current && isTerminalSessionStatus(current.status)) return current;
    await new Promise((resolve) => globalThis.setTimeout(resolve, 250));
    if (activeManagedSessionId !== sessionId) break;
    try {
      await hydrateManagedScanState();
    } catch {
      // The durable event path remains authoritative; a transient hydration failure is retried.
    }
  }
  throw new Error("Managed scan session was superseded.");
}

async function askForScanPath() {
  const t = currentT();
  const selectedPath = await open({
    directory: true,
    multiple: false,
    title: t("folderPickerTitle"),
    defaultPath: useScanManagerStore.getState().selectedFolders[0]
  });

  if (Array.isArray(selectedPath)) return selectedPath[0]?.trim() ?? "";
  return selectedPath?.trim() ?? "";
}

export const useScanManagerStore = create<ScanManagerStore>((set, get) => ({
  selectedFolders: [],
  defaultScanRoots: [],
  isScanning: false,
  isCancelingScan: false,
  scanState: initialScanState,
  activeScanSessionId: null,
  activeScanRunId: null,
  scanSession: null,
  scanRuns: [],
  lastRunRevision: 0,
  lastSessionRevision: 0,
  seenManagedEventIds: [],
  listenersRegistered: false,
  registrationPromise: null,
  unlisteners: [],
  initializeScanListeners: () => {
    if (get().listenersRegistered) return Promise.resolve();
    const registrationPromise = get().registrationPromise;
    if (registrationPromise) return registrationPromise;

    const promise = (async () => {
      try {
        try {
          await hydrateManagedScanState();
        } catch {
          // Listener registration remains available when optional startup
          // hydration is temporarily unavailable.
        }
        const unlistenGroup = await registerListenerGroup([
          () => tauriApi.onManagedScanEvent(async (event) => {
            if (event.parentSessionId !== activeManagedSessionId) return;
            const state = useScanManagerStore.getState();
            const previousRun = state.scanRuns.find((run) => run.id === event.runId);
            const decision = decideManagedScanEvent(
              event,
              activeManagedSessionId,
              previousRun?.revision,
              previousRun?.generation,
              state.lastSessionRevision,
              previousRun?.status,
              state.seenManagedEventIds
            );
            if (decision === "ignore") return;
            if (decision === "refresh") {
              try {
                await hydrateManagedScanState();
              } catch (error) {
                set((current) => ({
                  scanState: { ...current.scanState, error: readableError(error) }
                }));
              }
              return;
            }
            const runs = state.scanRuns.some((run) => run.id === event.runId)
              ? state.scanRuns.map((run) => run.id === event.runId ? projectRunFromEvent(run, event) : run)
              : [...state.scanRuns, projectRunFromEvent(undefined, event)];
            const session = state.scanSession
              ? sessionStatusFromMappings(
                  { ...state.scanSession, revision: Math.max(state.scanSession.revision, event.sessionRevision) },
                  runs
                )
              : null;
            const seenManagedEventIds = [...state.seenManagedEventIds, event.eventId].slice(-1000);
            set({
              activeScanRunId: terminalRunStatuses.has(event.status) ? state.activeScanRunId : event.runId,
              scanRuns: runs,
              scanSession: session,
              lastRunRevision: Math.max(state.lastRunRevision, event.runRevision),
              lastSessionRevision: Math.max(state.lastSessionRevision, event.sessionRevision),
              seenManagedEventIds,
              scanState: {
                ...state.scanState,
                status: scanStatusForBackendStatus(session?.status ?? event.status),
                progress: progressFromManagedEvent(event),
                error: event.errorMessage
              }
            });
          }),
          () => tauriApi.onScanProgress((progress) => {
            if (progress.jobId !== activeScanJobId) return;
            set((state) => ({
              scanState: {
                ...state.scanState,
                status: "scanning",
                progress,
                error: null
              }
            }));
          }),
          () => tauriApi.onScanBatch((batch: ScanBatchPayload) => {
            if (batch.jobId !== activeScanJobId) return;
            set((state) => ({
              scanState: {
                ...state.scanState,
                status: "scanning",
                progress: batch.progress,
                error: null
              }
            }));
          }),
          () => tauriApi.onScanComplete((summary: ScanSummary) => {
            if (summary.jobId !== activeScanJobId) return;
            set((state) => ({
              scanState: {
                ...state.scanState,
                status: "completed",
                progress: summary,
                error: null
              }
            }));
          }),
          () => tauriApi.onScanCanceled((summary: ScanSummary) => {
            if (summary.jobId !== activeScanJobId) return;
            set((state) => ({
              scanState: { ...state.scanState, status: "canceled", progress: summary, error: null }
            }));
          }),
          () => tauriApi.onScanError((payload) => {
            if (payload.jobId !== activeScanJobId) return;
            set((state) => ({
              scanState: {
                ...state.scanState,
                status: state.scanState.status === "idle" ? "scanning" : state.scanState.status,
                progress: state.scanState.progress
                  ? { ...state.scanState.progress, errors: state.scanState.progress.errors + 1 }
                  : {
                      root: payload.root,
                      jobId: payload.jobId,
                      jobKind: payload.jobKind,
                      scanned: 0,
                      files: 0,
                      directories: 0,
                      skipped: 0,
                      errors: 1,
                      elapsedMs: 0
                    },
                error: null
              }
            }));
          }),
          () => tauriApi.onDedupeProgress((payload) => {
            if (!isCurrentDedupeEvent(payload, activeDedupeParentScanJobId, activeDedupeJobId)) return;
            activeDedupeJobId ??= payload.dedupeJobId;
          }),
          () => tauriApi.onDedupeComplete((payload: DedupeCompletePayload) => {
            if (!isCurrentDedupeEvent(payload, activeDedupeParentScanJobId, activeDedupeJobId)) return;
            activeDedupeJobId = null;
            activeDedupeParentScanJobId = null;
            if (payload.status === "completed" || payload.status === "completed_with_warnings") {
              void useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery);
            }
          })
        ]);
        set({ listenersRegistered: true, registrationPromise: null, unlisteners: [unlistenGroup] });
      } catch (error) {
        set((state) => ({
          registrationPromise: null,
          scanState: { ...state.scanState, status: "error", error: readableError(error) }
        }));
        useAppStore.getState().showError(readableError(error));
      }
    })();
    set({ registrationPromise: promise });
    return promise;
  },
  refreshManagedScanState: async () => {
    await hydrateManagedScanState();
  },
  setDefaultScanRoots: (roots) => set({ defaultScanRoots: roots }),
  reset: () => {
    scanJobCanceled = false;
    activeScanJobId = null;
    activeManagedSessionId = null;
    activeManagedRequest = null;
    activeDedupeParentScanJobId = null;
    activeDedupeJobId = null;
    set({
      scanState: initialScanState,
      isScanning: false,
      isCancelingScan: false,
      activeScanSessionId: null,
      activeScanRunId: null,
      scanSession: null,
      scanRuns: [],
      lastRunRevision: 0,
      lastSessionRevision: 0,
      seenManagedEventIds: []
    });
  },
  scanPath: async (path) => {
    await get().scanPaths([path]);
  },
  scanPaths: async (paths) => {
    if (get().isScanning) return;

    const t = currentT();
    const scanRoots = paths.map((path) => path.trim()).filter(Boolean);
    if (!scanRoots.length) {
      useAppStore.getState().showError(t("noFolderSelected"));
      return;
    }

    await get().initializeScanListeners();
    scanJobCanceled = false;
    activeManagedSessionId = null;
    activeManagedRequest = {
      roots: scanRoots,
      requestKey: nextRequestKey(),
      dedupe: true
    };
    set({
      selectedFolders: scanRoots,
      isScanning: true,
      isCancelingScan: false,
      scanState: initialScanState,
      activeScanSessionId: null,
      activeScanRunId: null,
      scanSession: null,
      scanRuns: [],
      lastRunRevision: 0,
      lastSessionRevision: 0,
      seenManagedEventIds: []
    });

    try {
      const start = await tauriApi.startManagedScan(activeManagedRequest);
      activeManagedSessionId = start.session.id;
      activeDedupeParentScanJobId = start.session.id;
      activeDedupeJobId = null;
      applyManagedStartSnapshot(start);
      set((state) => ({
        scanState: {
          ...state.scanState,
          status: scanStatusForBackendStatus(start.session.status),
          progress: state.scanState.progress
        }
      }));
      if (scanJobCanceled) {
        const runToCancel = start.runs.find((run) =>
          ["queued", "running", "cancelling"].includes(run.status)
        );
        if (runToCancel) await tauriApi.cancelScanRun(runToCancel.id);
      }
      const session = await waitForManagedSession(start.session.id);
      // A root that ended in `requires_reconciliation` still persisted every file it
      // managed to observe, so its scope is valid and the library must refresh.
      // Excluding it here was what left the UI showing stale contents after a scan.
      const completedScanRoots = session.roots
        .filter((root) =>
          ["completed", "completed_with_warnings", "requires_reconciliation", "covered", "duplicate", "nested"].includes(
            root.status
          )
        )
        .map((root) => root.requestedPath)
        .filter((path, index, all) => all.indexOf(path) === index);
      const files = session.scannedFiles;
      const finalStatus = scanStatusForBackendStatus(session.status);
      const indexIncomplete = indexIsIncompleteForStatus(session.status);
      set((state) => ({
        scanState: { ...state.scanState, status: finalStatus, error: null },
        activeScanRunId: null,
        scanSession: session
      }));
      if (completedScanRoots.length) {
        useFileLibraryStore.getState().setCurrentScanScope(completedScanRoots, session.id);
        await useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery);
      }
      if (finalStatus === "canceled") {
        useAppStore.getState().showSuccess(t("scanCanceled"));
      } else if (finalStatus === "completed") {
        // The scan succeeded either way; when coverage was incomplete we say so
        // instead of silently dropping the signal, but we do not report a failure.
        useAppStore
          .getState()
          .showSuccess(
            indexIncomplete
              ? t("scanIndexIncomplete").replace("{files}", files.toLocaleString())
              : `${t("success")}: ${files.toLocaleString()} ${t("files")}`
          );
      } else if (finalStatus === "error") {
        useAppStore.getState().showError(session.errorMessage ?? t("unknown"));
      }
    } catch (error) {
      if (activeManagedSessionId === null && isScanAdmissionBusy(error)) {
        activeManagedRequest = null;
        set({ scanState: initialScanState });
        useAppStore.getState().showToast({ type: "info", message: t("scanAlreadyInProgress") });
        return;
      }
      const message = readableError(error);
      set((state) => ({
        scanState: { ...state.scanState, status: "error", error: message }
      }));
      useAppStore.getState().showError(message);
    } finally {
      activeScanJobId = null;
      set({ isScanning: false, isCancelingScan: false });
    }
  },
  handleScan: async () => {
    try {
      const { defaultScanRoots, scanPaths } = get();
      const defaultPaths = enabledScanRootPaths(defaultScanRoots);
      const paths = defaultPaths.length ? defaultPaths : [await askForScanPath()].filter(Boolean);
      await scanPaths(paths);
    } catch (error) {
      useAppStore.getState().showError(readableError(error));
    }
  },
  handleChooseFolders: async () => {
    try {
      const path = await askForScanPath();
      if (path) await get().scanPath(path);
    } catch (error) {
      useAppStore.getState().showError(readableError(error));
    }
  },
  cancelScan: async () => {
    if (!get().isScanning || get().isCancelingScan) return;
    scanJobCanceled = true;
    set((state) => ({
      isCancelingScan: true,
      scanState: { ...state.scanState, status: "scanning", error: null }
    }));
    try {
      const state = get();
      const activeRunId = state.activeScanRunId
        ?? state.scanRuns.find((run) => ["queued", "running", "cancelling"].includes(run.status))?.id;
      if (!activeRunId) return;
      await tauriApi.cancelScanRun(activeRunId);
    } catch (error) {
      scanJobCanceled = false;
      const message = readableError(error);
      set((state) => ({
        isCancelingScan: false,
        scanState: { ...state.scanState, status: "scanning", error: message }
      }));
      useAppStore.getState().showError(message);
    }
  }
}));
