import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  cloneFileQuerySpec,
  defaultFileLibraryQuerySpec,
  useFileLibraryInspectorStore,
  useFileLibrarySelectionStore,
  useFileLibraryQueryStore,
  useFileLibraryResultStore
} from "../src/store/useFileLibraryV2Store";
import { useFileLibraryStore, emptyStats } from "../src/store/useFileLibraryStore";
import type { FileQueryRequestV2, FileQueryResponseV2, FileQuerySpecV2 } from "../src/types/domain";

const api = vi.hoisted(() => ({
  query: vi.fn(),
  exactCount: vi.fn(),
  detail: vi.fn(),
  stats: vi.fn(),
  legacy: vi.fn()
}));

vi.mock("../src/api/tauriApi", () => ({
  tauriApi: {
    queryFileLibraryV2: api.query,
    resolveFileLibraryExactCountV2: api.exactCount,
    getFileLibraryDetail: api.detail,
    getStatsSummary: api.stats,
    getPagedFiles: api.legacy
  }
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function queryResponse(request: FileQueryRequestV2, overrides: Partial<FileQueryResponseV2> = {}): FileQueryResponseV2 {
  return {
    version: 2,
    requestId: request.requestId,
    queryFingerprint: `fingerprint-${request.query.text ?? "empty"}`,
    snapshotRevision: 1,
    files: [{
      id: request.query.text === "B" ? "file-b" : "file-a",
      name: `${request.query.text ?? "A"}.txt`,
      extension: "txt",
      displayDirectory: "C:/Data",
      size: 1,
      modifiedAt: 2,
      createdAt: 1,
      isDirectory: false,
      fileType: "Document",
      purpose: "Work",
      lifecycle: "Active",
      risk: "Normal",
      confidence: 1,
      isDuplicate: false,
      requiresReview: false,
      isStale: false,
      tags: [],
      tagCount: 0
    }],
    totalCount: 1,
    countState: "exact",
    countToken: null,
    nextCursor: null,
    hasMore: false,
    resultState: "complete",
    scopeHealth: { state: "healthy", roots: [], invalidReferences: [], message: null },
    ...overrides
  };
}

function resetStores() {
  useFileLibraryQueryStore.setState({
    spec: cloneFileQuerySpec(defaultFileLibraryQuerySpec),
    fingerprint: null,
    snapshotRevision: null,
    scopeHealth: null
  });
  useFileLibraryResultStore.getState().clear();
  useFileLibraryInspectorStore.getState().clear();
  useFileLibrarySelectionStore.getState().clear();
  api.query.mockReset();
  api.detail.mockReset();
  api.stats.mockResolvedValue(emptyStats);
  api.legacy.mockResolvedValue({ files: [], total: 0, limit: 50, offset: 0 });
  api.exactCount.mockReset();
}

async function flushPromises() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

beforeEach(resetStores);

describe("File Library result ownership", () => {
  it("refreshes the real Query V2 list, Details and sync health through the runtime watcher callback", async () => {
    let published = false;
    api.query.mockImplementation((request: FileQueryRequestV2) => {
      const response = queryResponse(request, {
        snapshotRevision: published ? 2 : 1,
        scopeHealth: { state: published ? "healthy" : "reconciliation_required", roots: [], invalidReferences: [], message: null }
      });
      response.files[0] = { ...response.files[0], size: published ? 57 : 10, modifiedAt: published ? 200 : 100 };
      return Promise.resolve(response);
    });
    api.detail.mockImplementation(async () => ({ id: "file-a", size: published ? 57 : 10, modifiedAt: published ? 200 : 100 }));
    await useFileLibraryResultStore.getState().loadFirstPage();
    await useFileLibraryInspectorStore.getState().loadDetail("file-a");
    expect(useFileLibraryResultStore.getState().files[0].size).toBe(10);
    expect(useFileLibraryInspectorStore.getState().detail?.size).toBe(10);
    published = true;
    // AppRuntimeProviders' useFsWatcher callback invokes this existing refresh.
    await useFileLibraryStore.getState().refresh();
    expect(useFileLibraryResultStore.getState().files[0]).toMatchObject({ size: 57, modifiedAt: 200 });
    expect(useFileLibraryInspectorStore.getState().detail).toMatchObject({ size: 57, modifiedAt: 200 });
    expect(useFileLibraryQueryStore.getState().scopeHealth?.state).toBe("healthy");
  });

  it("refreshes metadata and scope health even while the same query is in flight", async () => {
    const old = deferred<FileQueryResponseV2>();
    let oldRequest!: FileQueryRequestV2;
    api.query.mockImplementationOnce((request: FileQueryRequestV2) => {
      oldRequest = request;
      return old.promise;
    }).mockImplementation((request: FileQueryRequestV2) => {
      const response = queryResponse(request, { snapshotRevision: 2 });
      response.files[0] = { ...response.files[0], size: 57, modifiedAt: 200 };
      return Promise.resolve(response);
    });
    const pending = useFileLibraryResultStore.getState().loadFirstPage();
    await useFileLibraryResultStore.getState().refresh();
    expect(api.query).toHaveBeenCalledTimes(2);
    expect(useFileLibraryResultStore.getState().files[0]).toMatchObject({ size: 57, modifiedAt: 200 });
    expect(useFileLibraryQueryStore.getState().scopeHealth?.state).toBe("healthy");
    old.resolve(queryResponse(oldRequest, {
      scopeHealth: { state: "reconciliation_required", roots: [], invalidReferences: [], message: null }
    }));
    await pending;
    expect(useFileLibraryResultStore.getState().files[0]).toMatchObject({ size: 57, modifiedAt: 200 });
    expect(useFileLibraryQueryStore.getState().snapshotRevision).toBe(2);
    expect(useFileLibraryQueryStore.getState().scopeHealth?.state).toBe("healthy");
  });

  it("ignores a pending first page after clear", async () => {
    const pending = deferred<FileQueryResponseV2>();
    api.query.mockReturnValueOnce(pending.promise);
    const spec = { ...cloneFileQuerySpec(defaultFileLibraryQuerySpec), text: "A" };
    const load = useFileLibraryResultStore.getState().loadFirstPage(spec);

    useFileLibraryResultStore.getState().clear();
    pending.resolve(queryResponse({ version: 2, requestId: "a", query: spec, pageSize: 50, cursor: null }));
    await load;

    expect(useFileLibraryResultStore.getState().files).toEqual([]);
    expect(useFileLibraryResultStore.getState().isLoading).toBe(false);
    expect(useFileLibraryResultStore.getState().resultState).toBe("empty");
  });

  it("ignores a pending next page after clear", async () => {
    const pending = deferred<FileQueryResponseV2>();
    api.query.mockReturnValueOnce(pending.promise);
    useFileLibraryResultStore.setState({ files: [], nextCursor: "cursor-a", hasMore: true });
    const load = useFileLibraryResultStore.getState().loadNextPage();

    useFileLibraryResultStore.getState().clear();
    pending.resolve(queryResponse({ version: 2, requestId: "a-next", query: defaultFileLibraryQuerySpec, pageSize: 50, cursor: "cursor-a" }));
    await load;

    expect(useFileLibraryResultStore.getState().files).toEqual([]);
    expect(useFileLibraryResultStore.getState().nextCursor).toBeNull();
    expect(useFileLibraryResultStore.getState().hasMore).toBe(false);
  });

  it("ignores a deferred exact count after clear", async () => {
    const count = deferred<{ requestId: string; queryFingerprint: string; snapshotRevision: number; totalCount: number; countState: "exact" }>();
    api.query.mockResolvedValueOnce(queryResponse({ version: 2, requestId: "deferred", query: defaultFileLibraryQuerySpec, pageSize: 50, cursor: null }, {
      totalCount: null,
      countState: "deferred",
      countToken: "count-a"
    }));
    api.exactCount.mockReturnValueOnce(count.promise);
    await useFileLibraryResultStore.getState().loadFirstPage();
    expect(useFileLibraryResultStore.getState().isCountLoading).toBe(true);

    useFileLibraryResultStore.getState().clear();
    count.resolve({ requestId: "count-a", queryFingerprint: "fingerprint-empty", snapshotRevision: 1, totalCount: 99, countState: "exact" });
    await flushPromises();

    expect(useFileLibraryResultStore.getState().totalCount).toBe(0);
    expect(useFileLibraryResultStore.getState().isCountLoading).toBe(false);
    expect(useFileLibraryResultStore.getState().countToken).toBeNull();
  });

  it("keeps the new query when the cleared query resolves last", async () => {
    const oldResult = deferred<FileQueryResponseV2>();
    const newResult = deferred<FileQueryResponseV2>();
    api.query.mockReturnValueOnce(oldResult.promise).mockReturnValueOnce(newResult.promise);
    const oldSpec: FileQuerySpecV2 = { ...cloneFileQuerySpec(defaultFileLibraryQuerySpec), text: "A" };
    const newSpec: FileQuerySpecV2 = { ...cloneFileQuerySpec(defaultFileLibraryQuerySpec), text: "B" };
    const oldLoad = useFileLibraryResultStore.getState().loadFirstPage(oldSpec);

    useFileLibraryResultStore.getState().clear();
    const newLoad = useFileLibraryResultStore.getState().loadFirstPage(newSpec);
    newResult.resolve(queryResponse({ version: 2, requestId: "b", query: newSpec, pageSize: 50, cursor: null }));
    await newLoad;
    oldResult.resolve(queryResponse({ version: 2, requestId: "a", query: oldSpec, pageSize: 50, cursor: null }));
    await oldLoad;

    expect(useFileLibraryResultStore.getState().files.map((file) => file.id)).toEqual(["file-b"]);
  });

  it("does not let a cleared query error contaminate the new query", async () => {
    const oldResult = deferred<FileQueryResponseV2>();
    const newSpec: FileQuerySpecV2 = { ...cloneFileQuerySpec(defaultFileLibraryQuerySpec), text: "B" };
    api.query.mockReturnValueOnce(oldResult.promise).mockResolvedValueOnce(queryResponse({ version: 2, requestId: "b", query: newSpec, pageSize: 50, cursor: null }));
    const oldLoad = useFileLibraryResultStore.getState().loadFirstPage({ ...cloneFileQuerySpec(defaultFileLibraryQuerySpec), text: "A" });

    useFileLibraryResultStore.getState().clear();
    await useFileLibraryResultStore.getState().loadFirstPage(newSpec);
    oldResult.reject(new Error("old query failed"));
    await oldLoad;

    expect(useFileLibraryResultStore.getState().files.map((file) => file.id)).toEqual(["file-b"]);
    expect(useFileLibraryResultStore.getState().error).toBeNull();
    expect(useFileLibraryResultStore.getState().resultState).toBe("complete");
  });
});
