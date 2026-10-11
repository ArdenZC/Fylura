import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DashboardStats, FileQueryResult, LibraryFilter, LibraryScope } from "../src/types/domain";
import {
  LIBRARY_PAGE_SIZE,
  LIBRARY_SCOPE_STORAGE_KEY,
  emptyPage,
  emptyStats,
  readPersistedLibraryScope,
  useFileLibraryStore
} from "../src/store/useFileLibraryStore";
import { useScanManagerStore } from "../src/store/useScanManagerStore";

const apiMocks = vi.hoisted(() => ({
  startManagedScan: vi.fn(),
  getPagedFiles: vi.fn(),
  getStatsSummary: vi.fn(),
  dialogOpen: vi.fn()
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: apiMocks.dialogOpen
}));

vi.mock("../src/api/tauriApi", () => ({
  tauriApi: {
    startManagedScan: apiMocks.startManagedScan,
    getPagedFiles: apiMocks.getPagedFiles,
    getStatsSummary: apiMocks.getStatsSummary
  }
}));

function managedStart(roots: string[]) {
  const sessionId = "managed-session-test";
  const now = 1_750_000_000;
  const runs = roots.map((root, index) => ({
    id: `managed-run-${index}`,
    scanRootId: `managed-root-${index}`,
    rootPath: root,
    generation: 1,
    parentSessionId: sessionId,
    status: "completed",
    phase: "completed",
    scannedFiles: 1,
    scannedDirectories: 0,
    processedBytes: 1,
    warningsCount: 0,
    errorsCount: 0,
    metadataErrorCount: 0,
    coverageErrorCount: 0,
    coverageComplete: true,
    staleReconciliationAllowed: false,
    cancelRequested: false,
    revision: 4,
    sessionRevision: 4,
    startedAt: now - 1,
    finishedAt: now,
    lastCheckpointAt: now,
    errorCode: null,
    errorMessage: null,
    resultJson: null,
    createdAt: now - 1,
    updatedAt: now
  }));
  return {
    session: {
      id: sessionId,
      requestKey: "managed-request-test",
      canonicalRequestHash: "hash",
      status: "completed",
      phase: "completed",
      cancelRequested: false,
      requestedRootCount: roots.length,
      effectiveRootCount: roots.length,
      completedRootCount: roots.length,
      failedRootCount: 0,
      cancelledRootCount: 0,
      coveredRootCount: 0,
      unstartedRootCount: 0,
      dedupeRequested: true,
      dedupeDispatchState: "pending",
      dedupeAttemptCount: 0,
      dedupeJobId: null,
      dedupeLastError: null,
      scannedFiles: roots.length,
      scannedDirectories: 0,
      warningsCount: 0,
      errorsCount: 0,
      revision: 4,
      startedAt: now - 1,
      finishedAt: now,
      lastCheckpointAt: now,
      errorCode: null,
      errorMessage: null,
      resultJson: null,
      createdAt: now - 1,
      updatedAt: now,
      roots: roots.map((root, index) => ({
        sessionId,
        requestedIndex: index,
        requestedPath: root,
        normalizedRequestedPath: root,
        resolution: "effective",
        effectiveRootId: `managed-root-${index}`,
        effectivePath: root,
        effectiveIndex: index,
        runId: `managed-run-${index}`,
        status: "completed",
        reason: null,
        createdAt: now - 1,
        updatedAt: now
      }))
    },
    runs
  };
}

function stats(): DashboardStats {
  return { ...emptyStats };
}

function page(): FileQueryResult {
  return { ...emptyPage, files: [] };
}

function installLocalStorage() {
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: vi.fn((key: string) => store.get(key) ?? null),
      setItem: vi.fn((key: string, value: string) => {
        store.set(key, value);
      }),
      removeItem: vi.fn((key: string) => {
        store.delete(key);
      }),
      clear: vi.fn(() => {
        store.clear();
      })
    }
  });
  return globalThis.localStorage;
}

describe("library scope store", () => {
  beforeEach(() => {
    installLocalStorage();
    apiMocks.startManagedScan.mockReset().mockImplementation((request: { roots: string[] }) => Promise.resolve(managedStart(request.roots)));
    apiMocks.getPagedFiles.mockReset().mockResolvedValue(page());
    apiMocks.getStatsSummary.mockReset().mockResolvedValue(stats());
    useFileLibraryStore.setState({
      stats: emptyStats,
      libraryPage: emptyPage,
      selectedFileId: "",
      firstPageRequestId: 0,
      libraryFilter: "all" as LibraryFilter,
      scope: { kind: "current_scan", roots: [] }
    });
    useScanManagerStore.getState().reset();
    useScanManagerStore.setState({
      selectedFolders: [],
      isScanning: false,
      defaultScanRoots: [],
      listenersRegistered: true,
      registrationPromise: null
    });
  });

  it("sets current scan scope after scanPath succeeds", async () => {
    await useScanManagerStore.getState().scanPath("F:/Downloads");

    expect(useFileLibraryStore.getState().scope).toEqual({
      kind: "current_scan",
      roots: ["F:/Downloads"],
      scanSessionId: "managed-session-test"
    });
  });

  it("refresh carries the active scope to stats and paged files", async () => {
    const scope: LibraryScope = { kind: "roots", roots: ["F:/Projects"] };
    useFileLibraryStore.getState().setScope(scope);

    await useFileLibraryStore.getState().refresh("pdf");

    expect(apiMocks.getStatsSummary).toHaveBeenCalledWith(scope);
    expect(apiMocks.getPagedFiles).toHaveBeenCalledWith(LIBRARY_PAGE_SIZE, 0, "pdf", scope, undefined);
  });

  it("refresh carries the active library filter to paged files", async () => {
    const scope: LibraryScope = { kind: "roots", roots: ["F:/Projects"] };
    useFileLibraryStore.getState().setScope(scope);
    useFileLibraryStore.getState().setLibraryFilter("review");

    await useFileLibraryStore.getState().refresh("pdf");

    expect(apiMocks.getStatsSummary).toHaveBeenCalledWith(scope);
    expect(apiMocks.getPagedFiles).toHaveBeenCalledWith(LIBRARY_PAGE_SIZE, 0, "pdf", scope, {
      libraryFilter: "review"
    });
  });

  it("switches to all indexed files explicitly", () => {
    useFileLibraryStore.getState().setScope({ kind: "all" });

    expect(useFileLibraryStore.getState().scope.kind).toBe("all");
  });

  it("adopts configured Settings roots only when the saved Library scope is still empty", () => {
    const configuredRoot = {
      id: "downloads",
      path: "F:/Downloads",
      label: "Downloads",
      enabled: true,
      createdAt: "2026-06-22T00:00:00.000Z"
    };

    expect(useFileLibraryStore.getState().adoptConfiguredRootsIfScopeEmpty([configuredRoot])).toBe(true);
    expect(useFileLibraryStore.getState().scope).toEqual({ kind: "all" });
    expect(localStorage.setItem).toHaveBeenCalledWith(
      LIBRARY_SCOPE_STORAGE_KEY,
      JSON.stringify({ version: 1, scope: { kind: "all" } })
    );
    expect(readPersistedLibraryScope()).toEqual({ kind: "all" });

    const selectedScan = {
      kind: "current_scan" as const,
      roots: ["F:/Downloads"],
      scanSessionId: "completed-session"
    };
    useFileLibraryStore.getState().setScope(selectedScan);
    expect(useFileLibraryStore.getState().adoptConfiguredRootsIfScopeEmpty([configuredRoot])).toBe(false);
    expect(useFileLibraryStore.getState().scope).toEqual(selectedScan);

    const disabledRoot = { ...configuredRoot, enabled: false };
    useFileLibraryStore.getState().setScope({ kind: "current_scan", roots: [] });
    expect(useFileLibraryStore.getState().adoptConfiguredRootsIfScopeEmpty([disabledRoot])).toBe(false);
    expect(useFileLibraryStore.getState().scope).toEqual({ kind: "current_scan", roots: [] });
  });

  it("scan button scans enabled default roots without opening the folder picker", async () => {
    useScanManagerStore.setState({
      defaultScanRoots: [
        {
          id: "downloads",
          path: "F:/Downloads",
          label: "Downloads",
          enabled: true,
          createdAt: "2026-06-22T00:00:00.000Z"
        },
        {
          id: "archive",
          path: "D:/Archive",
          label: "Archive",
          enabled: false,
          createdAt: "2026-06-22T00:00:00.000Z"
        },
        {
          id: "projects",
          path: "D:/Projects",
          label: "Projects",
          enabled: true,
          createdAt: "2026-06-22T00:00:00.000Z"
        }
      ]
    });

    await useScanManagerStore.getState().handleScan();

    expect(apiMocks.dialogOpen).not.toHaveBeenCalled();
    expect(apiMocks.startManagedScan).toHaveBeenCalledWith(expect.objectContaining({
      roots: ["F:/Downloads", "D:/Projects"],
      dedupe: true,
      requestKey: expect.any(String)
    }));
    expect(useFileLibraryStore.getState().scope).toEqual({
      kind: "current_scan",
      roots: ["F:/Downloads", "D:/Projects"],
      scanSessionId: "managed-session-test"
    });
  });

  it("persists explicit scope changes to localStorage", () => {
    const scope: LibraryScope = { kind: "roots", roots: ["F:/Projects"] };

    useFileLibraryStore.getState().setScope(scope);

    expect(localStorage.setItem).toHaveBeenCalledWith(
      LIBRARY_SCOPE_STORAGE_KEY,
      JSON.stringify({ version: 1, scope })
    );
  });

  it("reads a persisted scope when localStorage has a valid scope", () => {
    const scope: LibraryScope = { kind: "current_scan", roots: ["F:/Downloads"] };
    localStorage.setItem(LIBRARY_SCOPE_STORAGE_KEY, JSON.stringify(scope));

    expect(readPersistedLibraryScope()).toEqual(scope);
  });
});
