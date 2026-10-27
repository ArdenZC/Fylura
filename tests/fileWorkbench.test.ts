import { describe, expect, it, vi } from "vitest";
import { FileWorkbenchModel } from "../src/views/fileLibrary/workbench/workbenchModel";
import { FileLibraryExperienceController } from "../src/views/fileLibrary/fileLibraryExperience";
import {
  FileWorkspaceController,
  type FileWorkspaceApi,
} from "../src/fileWorkspace";
import {
  createFileLibraryProjection,
  cloneFileQuerySpec,
  defaultFileLibraryQuerySpec,
} from "../src/store/useFileLibraryV2Store";
import { tauriApi } from "../src/api/tauriApi";
import type {
  FileLibraryDetail,
  FileQueryResponseV2,
} from "../src/types/domain";
import { pathCrumbs } from "../src/views/fileLibrary/workbench/PathBar";

const defer = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
};

describe("U2 independent workspace instances", () => {
  it("retains each tab and pane's controller, history presentation and selection, then disposes only the closed tab", async () => {
    const root = new FileLibraryExperienceController();
    const model = new FileWorkbenchModel(root);
    const first = model.getSnapshot().tabs[0];
    first.panes[0].controller.setViewMode("grid");
    first.panes[0].projection.useFileLibrarySelectionStore
      .getState()
      .setExplicit(["first-file"]);
    const second = model.newTab()!;
    const secondController = second.panes[0].controller;
    second.panes[0].projection.useFileLibrarySelectionStore
      .getState()
      .setExplicit(["other-file"]);
    model.toggleDual(second.id);
    const right = second.panes[1];
    right.controller.setViewMode("list");
    model.updatePath(right.id, "C:/Destination");
    model.activate(first.id);
    expect(
      first.panes[0].controller.getState().workspace.session.presentation
        .viewMode,
    ).toBe("grid");
    expect(
      first.panes[0].projection.useFileLibrarySelectionStore.getState()
        .selection,
    ).toEqual({ kind: "explicit", fileIds: ["first-file"] });
    model.activate(second.id, right.id);
    model.toggleDual(second.id);
    model.toggleDual(second.id);
    expect(second.panes[1]).toBe(right);
    expect(right.path).toBe("C:/Destination");
    expect(right.controller).not.toBe(secondController);
    const dispose = vi.spyOn(secondController, "dispose");
    await model.close(second.id);
    expect(dispose).toHaveBeenCalledOnce();
    expect(model.getSnapshot().activeTabId).toBe(first.id);
    expect(root.getState().workspace.session.disposed).toBe(false);
    await model.dispose();
    await root.dispose();
  });

  it("bounds tab ownership and always leaves a usable tab after closing the last one", async () => {
    const root = new FileLibraryExperienceController();
    const model = new FileWorkbenchModel(root);
    for (let i = 0; i < 7; i++) expect(model.newTab()).not.toBeNull();
    expect(model.newTab()).toBeNull();
    for (const tab of [...model.getSnapshot().tabs]) await model.close(tab.id);
    expect(model.getSnapshot().tabs).toHaveLength(1);
    expect(
      model.getSnapshot().tabs[0].panes[0].controller.getState().workspace
        .session.disposed,
    ).toBe(false);
    await model.dispose();
    await root.dispose();
  });

  it("isolates concurrent Query V2 responses, deferred detail publication and all-matching selection", async () => {
    const a = createFileLibraryProjection(),
      b = createFileLibraryProjection();
    const replyA = defer<FileQueryResponseV2>(),
      replyB = defer<FileQueryResponseV2>();
    const query = vi
      .spyOn(tauriApi, "queryFileLibraryV2")
      .mockReturnValueOnce(replyA.promise)
      .mockReturnValueOnce(replyB.promise);
    a.useFileLibraryQueryStore.getState().setSpec({
      ...cloneFileQuerySpec(defaultFileLibraryQuerySpec),
      text: "A",
    });
    b.useFileLibraryQueryStore.getState().setSpec({
      ...cloneFileQuerySpec(defaultFileLibraryQuerySpec),
      text: "B",
    });
    const loadA = a.useFileLibraryResultStore.getState().loadFirstPage(),
      loadB = b.useFileLibraryResultStore.getState().loadFirstPage();
    const response = (key: string): FileQueryResponseV2 => ({
      version: 2,
      requestId: key,
      queryFingerprint: key,
      snapshotRevision: 1,
      files: [],
      totalCount: 2,
      countState: "exact",
      countToken: null,
      nextCursor: null,
      hasMore: false,
      resultState: "complete",
      scopeHealth: {
        state: "healthy",
        roots: [],
        invalidReferences: [],
        message: null,
      },
    });
    replyB.resolve(response("B"));
    await loadB;
    replyA.resolve(response("A"));
    await loadA;
    expect(query.mock.calls[0][0].requestId).not.toBe(
      query.mock.calls[1][0].requestId,
    );
    a.useFileLibrarySelectionStore.getState().selectAllMatching();
    b.useFileLibrarySelectionStore.getState().setExplicit(["B-file"]);
    expect(a.useFileLibrarySelectionStore.getState().selection).toMatchObject({
      kind: "all_matching",
      queryFingerprint: "A",
    });
    expect(b.useFileLibrarySelectionStore.getState().selection).toMatchObject({
      kind: "explicit",
      fileIds: ["B-file"],
    });
    const old = defer<FileLibraryDetail>(),
      fresh = defer<FileLibraryDetail>();
    const detail = vi
      .spyOn(tauriApi, "getFileLibraryDetail")
      .mockReturnValueOnce(old.promise)
      .mockReturnValueOnce(fresh.promise);
    const readA = a.useFileLibraryInspectorStore
        .getState()
        .loadDetail("A-file"),
      readB = b.useFileLibraryInspectorStore.getState().loadDetail("B-file");
    fresh.resolve({ id: "B-file" } as FileLibraryDetail);
    await readB;
    old.resolve({ id: "A-file" } as FileLibraryDetail);
    await readA;
    expect(a.useFileLibraryInspectorStore.getState().detail?.id).toBe("A-file");
    expect(b.useFileLibraryInspectorStore.getState().detail?.id).toBe("B-file");
    query.mockRestore();
    detail.mockRestore();
  });

  it("pauses hidden-pane monitoring, rejects delayed observer publication, and retains the Browse session", async () => {
    const monitor = defer<{
      monitorId: string;
      sessionId: string;
      pathRef: { id: string };
    }>();
    const dispose = vi.fn(async () => undefined);
    const opened = {
      sessionId: "pane-session",
      rootPathRef: { id: "root" },
      location: {
        ref: {
          kind: "ephemeral",
          browseSessionId: "pane-session",
          locationId: "location",
        },
        displayName: "Folder",
        kind: "local",
        availability: "available",
        freshness: "fresh",
        capabilities: {
          canBrowse: true,
          canReadMetadata: true,
          canWatch: true,
          canPreview: true,
          canAddToLibrary: false,
          canRequestMaterialization: false,
        },
      },
    };
    const workspace = new FileWorkspaceController({
      browseOpen: async () => opened,
      changeStart: () => monitor.promise,
      changeDispose: dispose,
      browseDispose: async () => undefined,
      browseRetainPath: async () => undefined,
      browseReleasePath: async () => undefined,
    } as unknown as FileWorkspaceApi);
    await workspace.openBrowse({
      platform: "windows",
      routingHint: "C:/Folder",
    });
    const start = workspace.startChange({ id: "root" });
    await workspace.setObservationActive(false);
    monitor.resolve({
      monitorId: "late",
      sessionId: "pane-session",
      pathRef: { id: "root" },
    });
    await expect(start).resolves.toBeNull();
    expect(dispose).toHaveBeenCalledWith({ monitorId: "late" });
    expect(workspace.getState().browse?.sessionId).toBe("pane-session");
    expect(workspace.getState().session.currentTarget?.kind).toBe("browse");
    await workspace.setObservationActive(true);
    await workspace.dispose();
  });

  it("re-admits released path history with fresh session refs and preserves view state", async () => {
    let serial = 0;
    const opened = (path: string) => ({
      sessionId: `session-${++serial}`,
      rootPathRef: { id: `root-${serial}` },
      location: {
        ref: {
          kind: "ephemeral",
          browseSessionId: `session-${serial}`,
          locationId: `location-${serial}`,
        },
        displayName: path,
        kind: "local",
        availability: "available",
        freshness: "not_applicable",
        capabilities: {
          canBrowse: true,
          canReadMetadata: true,
          canWatch: false,
          canPreview: false,
          canAddToLibrary: false,
          canRequestMaterialization: false,
        },
      },
    });
    const dispose = vi.fn(async () => undefined);
    const restore = vi.fn(async (request: any) =>
      opened(request.locator.routingHint),
    );
    const api = {
      browseOpen: async (request: any) => opened(request.routingHint),
      browseRestore: restore,
      browseDispose: dispose,
      browseRetainPath: async () => undefined,
      browseReleasePath: async () => undefined,
    } as unknown as FileWorkspaceApi;
    const workspace = new FileWorkspaceController(api);
    const a = await workspace.openBrowse({
      platform: "windows",
      routingHint: "C:/A",
    });
    workspace.session.setPresentation({ viewMode: "grid" });
    await workspace.openBrowse({ platform: "windows", routingHint: "C:/B" });
    await workspace.releaseReadmittableHistorySessions();
    expect(dispose).toHaveBeenCalledWith({ sessionId: a!.sessionId });
    const restored = await workspace.back();
    expect(restore).toHaveBeenCalledWith(
      expect.objectContaining({
        locator: expect.objectContaining({ routingHint: "C:/A" }),
      }),
    );
    expect(restored?.sessionId).not.toBe(a?.sessionId);
    expect(workspace.getState().session.presentation.viewMode).toBe("grid");
    await workspace.releaseReadmittableHistorySessions();
    await workspace.forward();
    expect(workspace.getState().browse?.location.displayName).toBe("C:/B");
    await workspace.dispose();
  });

  it("retains opaque history without a safe restore locator", async () => {
    let serial = 0;
    const dispose = vi.fn(async () => undefined);
    const api = {
      browseOpen: async () => ({
        sessionId: `session-${++serial}`,
        rootPathRef: { id: `root-${serial}` },
        location: {
          ref: {
            kind: "ephemeral",
            browseSessionId: `session-${serial}`,
            locationId: `location-${serial}`,
          },
          displayName: "Opaque",
          capabilities: {},
        },
      }),
      browseDispose: dispose,
      browseRetainPath: async () => undefined,
      browseReleasePath: async () => undefined,
    } as unknown as FileWorkspaceApi;
    const workspace = new FileWorkspaceController(api);
    const opaque = await workspace.openBrowse({
      platform: "windows",
      routingHint: "",
      globalEntryId: "opaque-global-id",
    });
    await workspace.openBrowse({ platform: "windows", routingHint: "C:/B" });
    await workspace.releaseReadmittableHistorySessions();
    expect(dispose).not.toHaveBeenCalledWith({ sessionId: opaque!.sessionId });
    await workspace.dispose();
  });

  it("builds path-bar routing hints without treating them as access grants", () => {
    expect(pathCrumbs("C:\\Team\\Reports")).toEqual([
      { label: "C:", path: "C:/" },
      { label: "Team", path: "C:/Team" },
      { label: "Reports", path: "C:/Team/Reports" },
    ]);
    expect(pathCrumbs("/Users/zen").at(-1)?.path).toBe("/Users/zen");
  });
});
