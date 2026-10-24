import { beforeEach, describe, expect, it, vi } from "vitest";
import { useFileLibraryInspectorStore } from "../src/store/useFileLibraryV2Store";
import type { FileLibraryDetail } from "../src/types/domain";

const api = vi.hoisted(() => ({ getFileLibraryDetail: vi.fn() }));

vi.mock("../src/api/tauriApi", () => ({ tauriApi: api }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function detail(id: string, revision: number): FileLibraryDetail {
  return { id, revision } as unknown as FileLibraryDetail;
}

describe("File Library Inspector ownership", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    useFileLibraryInspectorStore.getState().clear();
    useFileLibraryInspectorStore.setState({ detail: null, selectionSummary: null, selectedId: null, requestEpoch: 0, isLoading: false, error: null });
  });

  it("supersedes pre-watcher detail reads when metadata is invalidated", async () => {
    const old = deferred<FileLibraryDetail>();
    const fresh = deferred<FileLibraryDetail>();
    api.getFileLibraryDetail.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
    const first = useFileLibraryInspectorStore.getState().loadDetail("file-a");
    const refresh = useFileLibraryInspectorStore.getState().loadDetail("file-a", true);
    expect(api.getFileLibraryDetail).toHaveBeenCalledTimes(2);
    fresh.resolve({ ...detail("file-a", 2), size: 57, modifiedAt: 200 });
    await refresh;
    old.resolve({ ...detail("file-a", 1), size: 10, modifiedAt: 100 });
    await first;
    expect(useFileLibraryInspectorStore.getState().detail).toMatchObject({ size: 57, modifiedAt: 200 });
  });

  it("commits a refresh only while the expected Inspector owner and epoch remain current", () => {
    useFileLibraryInspectorStore.setState({ selectedId: "file-a", requestEpoch: 7 });
    expect(useFileLibraryInspectorStore.getState().commitDetailIfCurrent("file-a", detail("file-a", 2), 7)).toBe(true);
    expect(useFileLibraryInspectorStore.getState().detail).toMatchObject({ id: "file-a", revision: 2 });

    useFileLibraryInspectorStore.setState({ selectedId: "file-b", requestEpoch: 8 });
    expect(useFileLibraryInspectorStore.getState().commitDetailIfCurrent("file-a", detail("file-a", 3), 7)).toBe(false);
    expect(useFileLibraryInspectorStore.getState().detail).toMatchObject({ id: "file-a", revision: 2 });
  });

  it("keeps B when a deferred A load resolves after the user switches selection", async () => {
    const pendingA = deferred<FileLibraryDetail>();
    api.getFileLibraryDetail.mockImplementation((fileId: string) => fileId === "file-a" ? pendingA.promise : Promise.resolve(detail("file-b", 1)));

    const loadA = useFileLibraryInspectorStore.getState().loadDetail("file-a");
    const loadB = useFileLibraryInspectorStore.getState().loadDetail("file-b");
    await loadB;
    pendingA.resolve(detail("file-a", 2));
    await loadA;

    expect(useFileLibraryInspectorStore.getState().selectedId).toBe("file-b");
    expect(useFileLibraryInspectorStore.getState().detail).toMatchObject({ id: "file-b", revision: 1 });
  });

  it("coalesces concurrent requests for the same current Inspector owner", async () => {
    const pending = deferred<FileLibraryDetail>();
    api.getFileLibraryDetail.mockReturnValue(pending.promise);

    const firstLoad = useFileLibraryInspectorStore.getState().loadDetail("file-a");
    const secondLoad = useFileLibraryInspectorStore.getState().loadDetail("file-a");
    expect(api.getFileLibraryDetail).toHaveBeenCalledOnce();

    pending.resolve(detail("file-a", 3));
    await expect(firstLoad).resolves.toMatchObject({ status: "applied", detail: { id: "file-a", revision: 3 } });
    await expect(secondLoad).resolves.toMatchObject({ status: "applied" });
    expect(useFileLibraryInspectorStore.getState().detail).toMatchObject({ id: "file-a", revision: 3 });
  });

  it("returns a visible failed outcome for the current Inspector owner", async () => {
    api.getFileLibraryDetail.mockRejectedValueOnce(new Error("detail_failed"));

    const outcome = await useFileLibraryInspectorStore.getState().loadDetail("file-a");

    expect(outcome).toMatchObject({ status: "failed", error: "detail_failed" });
    expect(useFileLibraryInspectorStore.getState().isLoading).toBe(false);
    expect(useFileLibraryInspectorStore.getState().error).toBe("detail_failed");
  });

  it("starts A2 after A to B to A and ignores the stale A1 error", async () => {
    const firstA = deferred<FileLibraryDetail>();
    const pendingB = deferred<FileLibraryDetail>();
    const secondA = deferred<FileLibraryDetail>();
    let aCalls = 0;
    api.getFileLibraryDetail.mockImplementation((fileId: string) => {
      if (fileId === "file-b") return pendingB.promise;
      aCalls += 1;
      return aCalls === 1 ? firstA.promise : secondA.promise;
    });

    const loadA1 = useFileLibraryInspectorStore.getState().loadDetail("file-a");
    const loadB = useFileLibraryInspectorStore.getState().loadDetail("file-b");
    pendingB.resolve(detail("file-b", 1));
    await loadB;
    const loadA2 = useFileLibraryInspectorStore.getState().loadDetail("file-a");
    expect(api.getFileLibraryDetail).toHaveBeenNthCalledWith(1, "file-a");
    expect(api.getFileLibraryDetail).toHaveBeenNthCalledWith(2, "file-b");
    expect(api.getFileLibraryDetail).toHaveBeenNthCalledWith(3, "file-a");

    firstA.reject(new Error("stale_a1_failed"));
    await expect(loadA1).resolves.toMatchObject({ status: "superseded" });
    secondA.resolve(detail("file-a", 2));
    await expect(loadA2).resolves.toMatchObject({ status: "applied", detail: { id: "file-a", revision: 2 } });
    expect(useFileLibraryInspectorStore.getState().selectedId).toBe("file-a");
    expect(useFileLibraryInspectorStore.getState().detail).toMatchObject({ id: "file-a", revision: 2 });
    expect(useFileLibraryInspectorStore.getState().error).toBeNull();
  });

  it("stays empty after clear even when the owned detail request resolves", async () => {

    const pendingClear = deferred<FileLibraryDetail>();
    api.getFileLibraryDetail.mockReturnValueOnce(pendingClear.promise);
    const loadAfterClear = useFileLibraryInspectorStore.getState().loadDetail("file-a");
    const epochBeforeClear = useFileLibraryInspectorStore.getState().requestEpoch;
    useFileLibraryInspectorStore.getState().clear();
    expect(useFileLibraryInspectorStore.getState().requestEpoch).toBe(epochBeforeClear + 1);
    pendingClear.resolve(detail("file-a", 4));
    await loadAfterClear;
    expect(useFileLibraryInspectorStore.getState().selectedId).toBeNull();
    expect(useFileLibraryInspectorStore.getState().detail).toBeNull();
  });
});
