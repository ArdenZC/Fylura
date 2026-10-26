import { describe, expect, it, vi } from "vitest";
import {
  FileWorkspaceController,
  type FileWorkspaceApi,
  type FileWorkspaceControllerState
} from "../src/fileWorkspace";
import { mockFileWorkspaceInvoke } from "../src/api/fileWorkspaceMockApi";
import type { BrowsePage } from "../src/types/fileWorkspace";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function fakeApi(overrides: Partial<FileWorkspaceApi> = {}): FileWorkspaceApi {
  const emptyPage = async (): Promise<BrowsePage> => ({
    sessionId: "session",
    requestId: "request",
    enumerationId: "enumeration",
    entries: [],
    completion: "complete"
  });
  return {
    browseOpen: async () => ({
      sessionId: "session",
      location: {
        ref: { kind: "ephemeral", browseSessionId: "session", locationId: "location" },
        displayName: "Browse",
        kind: "unknown",
        availability: "unknown",
        freshness: "not_applicable",
        capabilities: {
          canBrowse: false,
          canReadMetadata: false,
          canPreview: false,
          canWatch: false,
          canRequestMaterialization: false,
          canAddToLibrary: false
        }
      },
      rootPathRef: { id: "root" }
    }),
    browseRestore: async () => { throw new Error("unused"); },
    locationBrowse: async () => { throw new Error("unused"); },
    browseStartEnumeration: emptyPage,
    browseNextPage: emptyPage,
    browseCancel: async () => undefined,
    browseReleasePage: async () => undefined,
    browseReleasePath: async () => undefined,
    browseRetainPath: async () => undefined,
    browseDispose: async () => undefined,
    locationList: async () => [],
    changeStart: async () => ({ monitorId: "monitor", sessionId: "session", pathRef: { id: "root" } }),
    changePending: async () => null,
    changeRefresh: emptyPage,
    changeDispose: async () => undefined,
    readEligibility: async ({ source }) => ({ source, eligibility: "eligible" }),
    thumbnailRequest: async () => ({ cacheKey: "mock", bytes: new Uint8Array() }),
    thumbnailCancel: async () => true,
    previewCreate: async () => { throw new Error("unused"); },
    previewSnapshot: async () => { throw new Error("unused"); },
    previewStart: async () => { throw new Error("unused"); },
    previewCancel: async () => true,
    previewDispose: async () => true,
    previewSwitchSource: async () => { throw new Error("unused"); },
    previewAssetRequest: async () => { throw new Error("preview asset unused"); },
    ...overrides
  };
}

describe("W1-10 File Workspace integration", () => {
  it("rejects a late Browse admission after WorkspaceSession navigation", async () => {
    const admission = deferred<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>();
    const dispose = vi.fn(async () => undefined);
    const controller = new FileWorkspaceController(fakeApi({
      browseOpen: () => admission.promise,
      browseDispose: dispose
    }));
    const result = controller.openBrowse({
      platform: "windows",
      routingHint: "C:/Documents"
    });

    controller.navigate({ kind: "library", source: "search", key: "latest" });
    admission.resolve((await fakeApi().browseOpen({ platform: "windows", routingHint: "C:/Documents" })));

    await expect(result).resolves.toBeNull();
    expect(dispose).toHaveBeenCalledWith({ sessionId: "session" });
    expect(controller.getState().session.currentTarget?.kind).toBe("library");
  });

  it("adopts an opaque LocationRef into fresh Browse refs without a renderer restore path", async () => {
    const location = { kind: "managed" as const, scanRootId: "managed-root" };
    const locationBrowse = vi.fn(async () => ({
      sessionId: "location-session",
      location: {
        ref: {
          kind: "ephemeral" as const,
          browseSessionId: "location-session",
          locationId: "fresh-location"
        },
        displayName: "Backend location",
        kind: "unknown" as const,
        availability: "available" as const,
        freshness: "not_applicable" as const,
        capabilities: {
          canBrowse: true,
          canReadMetadata: false,
          canPreview: false,
          canWatch: false,
          canRequestMaterialization: false,
          canAddToLibrary: false
        }
      },
      rootPathRef: { id: "fresh-root" }
    }));
    const controller = new FileWorkspaceController(fakeApi({ locationBrowse }));

    const response = await controller.browseLocation(location);

    expect(locationBrowse).toHaveBeenCalledWith({ location });
    expect(response?.location.capabilities).toEqual(expect.objectContaining({
      canBrowse: true,
      canReadMetadata: false,
      canPreview: false,
      canWatch: false,
      canRequestMaterialization: false,
      canAddToLibrary: false
    }));
    expect(controller.getState().session.currentTarget).toEqual({
      kind: "browse",
      location: response!.location.ref,
      pathRef: response!.rootPathRef
    });
    expect(controller.session.serializeRestoreLocator()).toBeNull();

    await controller.dispose();
  });

  it("cancels a late Thumbnail publication after navigation", async () => {
    const thumbnail = deferred<{ cacheKey: string; bytes: Uint8Array }>();
    const cancel = vi.fn(async () => true);
    const controller = new FileWorkspaceController(fakeApi({
      thumbnailRequest: () => thumbnail.promise,
      thumbnailCancel: cancel
    }));
    const request = controller.requestThumbnail({
      requestId: "thumbnail-1",
      source: { kind: "managed", fileId: "file-1" },
      variant: "small",
      workClass: "interactive"
    });

    controller.navigate({ kind: "library", source: "search", key: "latest" });
    thumbnail.resolve({ cacheKey: "logical-key", bytes: new Uint8Array([1, 2]) });

    await expect(request).resolves.toBeNull();
    expect(cancel).toHaveBeenCalledWith({ requestId: "thumbnail-1" });
  });

  it("cancels one active thumbnail through the presentation-owned seam", async () => {
    const thumbnail = deferred<{ cacheKey: string; bytes: Uint8Array }>();
    const cancel = vi.fn(async () => true);
    const controller = new FileWorkspaceController(fakeApi({
      thumbnailRequest: () => thumbnail.promise,
      thumbnailCancel: cancel
    }));
    const pending = controller.requestThumbnail({
      requestId: "thumbnail-active",
      source: { kind: "managed", fileId: "file-1" },
      variant: "medium",
      workClass: "interactive"
    });

    await expect(controller.cancelThumbnail("thumbnail-active")).resolves.toBe(true);
    expect(cancel).toHaveBeenCalledWith({ requestId: "thumbnail-active" });
    thumbnail.resolve({ cacheKey: "logical-key", bytes: new Uint8Array([1]) });
    await expect(pending).resolves.toEqual({ cacheKey: "logical-key", bytes: new Uint8Array([1]) });
  });

  it("cancels a pending Browse start by request identity during target teardown", async () => {
    const page = deferred<BrowsePage>();
    const browseCancel = vi.fn(async () => undefined);
    const browseReleasePage = vi.fn(async () => undefined);
    const controller = new FileWorkspaceController(fakeApi({
      browseStartEnumeration: () => page.promise,
      browseCancel,
      browseReleasePage
    }));
    await controller.openBrowse({ platform: "windows", routingHint: "C:/Documents" });
    const pending = controller.startEnumeration(undefined, "pending-enumeration", 1);

    controller.navigate({ kind: "library", source: "search", key: "latest" });
    await Promise.resolve();
    await Promise.resolve();
    expect(browseCancel).toHaveBeenCalledWith({
      sessionId: "session",
      requestId: "pending-enumeration"
    });

    page.resolve({
      sessionId: "session",
      requestId: "pending-enumeration",
      enumerationId: "enumeration-pending",
      entries: [],
      completion: "complete"
    });
    await expect(pending).resolves.toBeNull();
    expect(browseReleasePage).toHaveBeenCalled();
  });

  it("keeps Browse pages in controller state without a Query V2 store", async () => {
    const states: FileWorkspaceControllerState[] = [];
    const page: BrowsePage = {
      sessionId: "session",
      requestId: "request",
      enumerationId: "enumeration",
      entries: [],
      completion: "complete"
    };
    const controller = new FileWorkspaceController(fakeApi({
      browseStartEnumeration: async () => page
    }));
    controller.subscribe((state) => states.push(state));
    await controller.openBrowse({ platform: "windows", routingHint: "C:/Documents" });
    await controller.startEnumeration();

    expect(controller.getState().page).toEqual(page);
    expect(states.at(-1)?.page).toEqual(page);
  });

  it("sends the source-owned current-folder query with every new enumeration", async () => {
    const requests: Array<{ query: unknown }> = [];
    const controller = new FileWorkspaceController(fakeApi({
      browseStartEnumeration: async (request) => {
        requests.push({ query: request.query });
        return {
          sessionId: request.sessionId,
          requestId: request.requestId,
          enumerationId: `enumeration-${requests.length}`,
          entries: [],
          completion: "complete"
        };
      }
    }));

    await controller.openBrowse({ platform: "windows", routingHint: "C:/query" });
    await controller.startEnumeration(undefined, "query-directory", 8, {
      text: "sentinel",
      entryKind: "directory"
    });

    expect(requests).toEqual([{ query: { text: "sentinel", entryKind: "directory" } }]);
  });

  it("retains every published page batch until target teardown", async () => {
    const page1: BrowsePage = {
      sessionId: "session",
      requestId: "same-request",
      enumerationId: "same-enumeration",
      entries: [{
        ref: { kind: "ephemeral", browseSessionId: "session", entryId: "entry-1" },
        name: "one.txt",
        displayPath: "one.txt",
        kind: "file",
        materialization: "unknown"
      }],
      nextCursor: "page-2",
      completion: "partial"
    };
    const page2: BrowsePage = {
      sessionId: "session",
      requestId: "same-request",
      enumerationId: "same-enumeration",
      entries: [{
        ref: { kind: "ephemeral", browseSessionId: "session", entryId: "entry-2" },
        name: "two.txt",
        displayPath: "two.txt",
        kind: "file",
        materialization: "unknown"
      }],
      completion: "complete",
      knownCount: 2
    };
    const browseReleasePage = vi.fn(async () => undefined);
    const controller = new FileWorkspaceController(fakeApi({
      browseStartEnumeration: async () => page1,
      browseNextPage: async () => page2,
      browseReleasePage
    }));

    await controller.openBrowse({ platform: "windows", routingHint: "C:/paged" });
    await controller.startEnumeration(undefined, "same-request", 1);
    await controller.nextPage(1);

    expect(controller.getState().page).toEqual(page2);
    expect(browseReleasePage).not.toHaveBeenCalled();

    await controller.dispose();
    expect(browseReleasePage).toHaveBeenCalledTimes(2);
    expect(browseReleasePage).toHaveBeenCalledWith({ page: page1 });
    expect(browseReleasePage).toHaveBeenCalledWith({ page: page2 });
  });

  it("advances a 100k logical Browse workload with one bounded page owner", async () => {
    const totalEntries = 100_000;
    const pageSize = 100;
    const pageCount = totalEntries / pageSize;
    const pageFor = (pageIndex: number): BrowsePage => {
      const firstEntry = pageIndex * pageSize;
      const entries = Array.from({ length: pageSize }, (_, offset) => {
        const entryIndex = firstEntry + offset;
        return {
          ref: {
            kind: "ephemeral" as const,
            browseSessionId: "session",
            entryId: `entry-${entryIndex}`
          },
          name: `file-${entryIndex}.bin`,
          displayPath: `file-${entryIndex}.bin`,
          kind: "file" as const,
          materialization: "unknown" as const
        };
      });
      return {
        sessionId: "session",
        requestId: "logical-100k",
        enumerationId: "logical-100k-enumeration",
        entries,
        ...(pageIndex + 1 < pageCount ? { nextCursor: String(pageIndex + 1), completion: "partial" as const } : { completion: "complete" as const, knownCount: totalEntries })
      };
    };
    const browseReleasePage = vi.fn(async () => undefined);
    const controller = new FileWorkspaceController(fakeApi({
      browseStartEnumeration: async () => pageFor(0),
      browseNextPage: async ({ cursor }) => pageFor(Number(cursor)),
      browseReleasePage
    }));

    await controller.openBrowse({ platform: "windows", routingHint: "C:/logical-100k" });
    const first = await controller.startEnumeration(undefined, "logical-100k", pageSize);
    expect(first?.entries).toHaveLength(pageSize);
    expect(first?.nextCursor).toBe("1");
    for (let pageIndex = 1; pageIndex < pageCount; pageIndex += 1) {
      await controller.nextPage(pageSize);
    }

    expect(controller.getState().page?.entries).toHaveLength(pageSize);
    expect(controller.getState().page?.entries.at(-1)?.name).toBe("file-99999.bin");
    expect(browseReleasePage).not.toHaveBeenCalled();

    await controller.dispose();
    expect(browseReleasePage).toHaveBeenCalledTimes(pageCount);
  });

  it("reuses one live Browse session for nested history and bounds truncation cleanup", async () => {
    const response = await fakeApi().browseOpen({ platform: "windows", routingHint: "C:/nested" });
    const nestedPath = { id: "nested-path" };
    const deeperPath = { id: "deeper-path" };
    const activePaths = new Set([response.rootPathRef.id]);
    const retainedPaths: string[] = [];
    const releasedPaths: string[] = [];
    const startRequests: Array<{ sessionId: string; pathRef: { id: string } }> = [];
    const browseRestore = vi.fn(async () => { throw new Error("live-history-must-not-restore"); });
    const pageFor = (pathId: string, requestId: string): BrowsePage => {
      const pathRef = pathId === response.rootPathRef.id
        ? nestedPath
        : pathId === nestedPath.id ? deeperPath : undefined;
      if (pathRef !== undefined) activePaths.add(pathRef.id);
      return {
        sessionId: response.sessionId,
        requestId,
        enumerationId: `enumeration-${requestId}`,
        entries: pathRef === undefined ? [] : [{
          ref: {
            kind: "ephemeral",
            browseSessionId: response.sessionId,
            entryId: `entry-${requestId}`
          },
          pathRef,
          name: pathRef.id,
          displayPath: pathRef.id,
          kind: "directory",
          materialization: "unknown"
        }],
        completion: "complete"
      };
    };
    const browseReleasePath = vi.fn(async ({ pathRef }: { pathRef: { id: string } }) => {
      activePaths.delete(pathRef.id);
      releasedPaths.push(pathRef.id);
    });
    const browseDispose = vi.fn(async () => undefined);
    const controller = new FileWorkspaceController(fakeApi({
      browseOpen: async () => response,
      browseRestore,
      browseStartEnumeration: async (request) => {
        if (!activePaths.has(request.pathRef.id)) throw new Error("path_ref_not_live");
        startRequests.push({ sessionId: request.sessionId, pathRef: request.pathRef });
        return pageFor(request.pathRef.id, request.requestId);
      },
      browseRetainPath: async ({ pathRef }) => {
        if (!activePaths.has(pathRef.id)) throw new Error("path_ref_not_live");
        retainedPaths.push(pathRef.id);
      },
      browseReleasePath,
      browseDispose
    }));

    const opened = await controller.openBrowse({ platform: "windows", routingHint: "C:/nested" });
    const rootPage = await controller.startEnumeration(undefined, "root", 1);
    const rootTarget = {
      kind: "browse" as const,
      location: opened!.location.ref,
      pathRef: opened!.rootPathRef
    };
    expect(rootPage?.entries[0]?.pathRef).toEqual(nestedPath);

    controller.navigate({ kind: "browse", location: rootTarget.location, pathRef: nestedPath });
    const nestedPage = await controller.startEnumeration(nestedPath, "nested", 1);
    expect(nestedPage?.entries[0]?.pathRef).toEqual(deeperPath);

    controller.navigate({ kind: "browse", location: rootTarget.location, pathRef: deeperPath });
    await controller.startEnumeration(deeperPath, "deeper", 1);

    const back = await controller.back();
    expect(back?.sessionId).toBe(opened?.sessionId);
    expect(controller.getState().session.currentTarget).toEqual({
      kind: "browse",
      location: opened!.location.ref,
      pathRef: nestedPath
    });
    await controller.startEnumeration(undefined, "nested-back", 1);

    const forward = await controller.forward();
    expect(forward?.sessionId).toBe(opened?.sessionId);
    expect(controller.getState().session.currentTarget).toEqual({
      kind: "browse",
      location: opened!.location.ref,
      pathRef: deeperPath
    });
    await controller.startEnumeration(undefined, "deeper-forward", 1);
    expect(browseRestore).not.toHaveBeenCalled();
    expect(startRequests.every(({ sessionId }) => sessionId === opened!.sessionId)).toBe(true);
    expect(retainedPaths).toEqual(expect.arrayContaining([nestedPath.id, deeperPath.id]));

    controller.navigate({ kind: "library", source: "search", key: "library" });
    const fromLibrary = await controller.back();
    expect(fromLibrary?.sessionId).toBe(opened?.sessionId);
    expect(controller.getState().session.currentTarget?.kind).toBe("browse");
    await controller.forward();
    expect(controller.getState().browse).toBeNull();

    // Move back to the nested target, then navigate elsewhere. The deeper
    // forward entry is truncated and its path is released, while the shared
    // Browse session remains owned by root/nested history.
    await controller.back();
    await controller.back();
    controller.navigate({ kind: "library", source: "custom", key: "replacement" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(releasedPaths).toContain(deeperPath.id);
    expect(browseDispose).not.toHaveBeenCalled();

    await controller.dispose();
    expect(browseDispose).toHaveBeenCalledWith({ sessionId: opened!.sessionId });
    expect(activePaths.size).toBe(0);
  });

  it("releases every current-target handle on a rapid target switch", async () => {
    let nextSession = 0;
    const responseFor = (sessionId: string) => ({
      sessionId,
      location: {
        ref: { kind: "ephemeral" as const, browseSessionId: sessionId, locationId: `location-${sessionId}` },
        displayName: "Browse",
        kind: "unknown" as const,
        availability: "unknown" as const,
        freshness: "not_applicable" as const,
        capabilities: {
          canBrowse: false,
          canReadMetadata: false,
          canPreview: false,
          canWatch: false,
          canRequestMaterialization: false,
          canAddToLibrary: false
        }
      },
      rootPathRef: { id: `root-${sessionId}` }
    });
    const pageFor = (sessionId: string, requestId: string): BrowsePage => ({
      sessionId,
      requestId,
      enumerationId: `enumeration-${sessionId}`,
      entries: [],
      completion: "complete"
    });
    const thumbnail = deferred<{ cacheKey: string; bytes: Uint8Array }>();
    const browseDispose = vi.fn(async () => undefined);
    const browseReleasePage = vi.fn(async () => undefined);
    const browseReleasePath = vi.fn(async () => undefined);
    const changeDispose = vi.fn(async () => undefined);
    const thumbnailCancel = vi.fn(async () => true);
    const previewCancel = vi.fn(async () => true);
    const previewDispose = vi.fn(async () => true);
    const controller = new FileWorkspaceController(fakeApi({
      browseOpen: async () => responseFor(`session-${++nextSession}`),
      browseStartEnumeration: async ({ sessionId, requestId }) => pageFor(sessionId, requestId),
      browseReleasePage,
      browseReleasePath,
      browseDispose,
      changeStart: async ({ sessionId, pathRef }) => ({ monitorId: `monitor-${sessionId}`, sessionId, pathRef }),
      changeDispose,
      thumbnailRequest: () => thumbnail.promise,
      thumbnailCancel,
      previewCreate: async ({ requestId, source, hostKind }) => ({
        previewId: "preview-1",
        sessionId: "preview-1",
        requestId,
        source,
        hostKind,
        state: "idle",
        effectiveCapabilities: {
          canSearch: false,
          canZoom: false,
          canPlayback: false,
          canSelectText: false,
          canNavigateInternal: false,
          canNavigateSiblings: false,
          canOpenExternal: true,
          canReveal: true,
          canRequestMaterialization: true
        }
      }),
      previewCancel,
      previewDispose
    }));

    const first = await controller.openBrowse({ platform: "windows", routingHint: "C:/one" });
    expect(first).not.toBeNull();
    await controller.startEnumeration(undefined, "enumeration-request", 10);
    await controller.startChange({ id: `root-${first!.sessionId}` });
    await controller.createPreview({
      requestId: "preview-request",
      source: { kind: "managed", fileId: "file-1" },
      hostKind: "zen_floating"
    });
    const pendingThumbnail = controller.requestThumbnail({
      requestId: "thumbnail-request",
      source: { kind: "managed", fileId: "file-1" },
      variant: "small",
      workClass: "interactive"
    });

    const second = await controller.openBrowse({ platform: "windows", routingHint: "C:/two" });
    expect(second?.sessionId).not.toBe(first?.sessionId);
    expect(browseReleasePage).toHaveBeenCalledWith(expect.objectContaining({
      page: expect.objectContaining({ sessionId: first!.sessionId })
    }));
    expect(browseReleasePath).not.toHaveBeenCalled();
    expect(changeDispose).toHaveBeenCalledWith({ monitorId: `monitor-${first!.sessionId}` });
    expect(thumbnailCancel).toHaveBeenCalledWith({ requestId: "thumbnail-request" });
    expect(previewCancel).toHaveBeenCalledWith({ previewId: "preview-1" });
    expect(previewDispose).toHaveBeenCalledWith({ previewId: "preview-1" });
    expect(browseDispose).not.toHaveBeenCalled();

    const third = await controller.openBrowse({ platform: "windows", routingHint: "C:/three" });
    expect(third?.sessionId).not.toBe(second?.sessionId);
    expect(browseDispose).not.toHaveBeenCalled();

    thumbnail.resolve({ cacheKey: "late", bytes: new Uint8Array([1]) });
    await expect(pendingThumbnail).resolves.toBeNull();
    await controller.dispose();
    expect(browseDispose).toHaveBeenCalledWith({ sessionId: first!.sessionId });
    expect(browseDispose).toHaveBeenCalledWith({ sessionId: second!.sessionId });
    expect(browseDispose).toHaveBeenCalledWith({ sessionId: third!.sessionId });
  });

  it("keeps live Browse history refs across Library Back/Forward", async () => {
    const first = {
      ...(await fakeApi().browseOpen({ platform: "windows", routingHint: "C:/history" })),
      sessionId: "history-first",
      location: {
        ...(await fakeApi().browseOpen({ platform: "windows", routingHint: "C:/history" })).location,
        ref: { kind: "ephemeral" as const, browseSessionId: "history-first", locationId: "location-first" }
      },
      rootPathRef: { id: "root-history-first" }
    };
    const browseDispose = vi.fn(async () => undefined);
    const browseRestore = vi.fn(async () => { throw new Error("live-history-must-not-restore"); });
    const controller = new FileWorkspaceController(fakeApi({
      browseOpen: async () => first,
      browseRestore,
      browseDispose
    }));

    await controller.openBrowse({ platform: "windows", routingHint: "C:/history" });
    controller.navigate({ kind: "library", source: "search", key: "history" });
    const back = await controller.back();
    expect(back?.sessionId).toBe("history-first");
    expect(controller.getState().session.currentTarget).toEqual({
      kind: "browse",
      location: first.location.ref,
      pathRef: first.rootPathRef
    });
    expect(browseRestore).not.toHaveBeenCalled();
    expect(browseDispose).not.toHaveBeenCalled();

    await controller.forward();
    expect(controller.getState().browse).toBeNull();
    await controller.dispose();
    expect(browseDispose).toHaveBeenCalledWith({ sessionId: "history-first" });
  });
});

describe("File Workspace browser mock", () => {
  it("keeps Location -> Browse admission opaque, fresh, and fail-closed", async () => {
    const [managed] = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["locationList"]>>>(
      "file_workspace_location_list"
    );
    const managedAdmission = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["locationBrowse"]>>>(
      "file_workspace_location_browse",
      { request: { location: managed!.ref } }
    );
    expect(managedAdmission.location.kind).toBe("unknown");
    expect(managedAdmission.location.availability).toBe("available");
    expect(managedAdmission.location.capabilities).toEqual({
      canBrowse: true,
      canReadMetadata: false,
      canPreview: false,
      canWatch: false,
      canRequestMaterialization: false,
      canAddToLibrary: false
    });

    const source = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>(
      "file_workspace_browse_open",
      { request: { platform: "windows", routingHint: "C:/location-source", displayHint: "Source" } }
    );
    expect(source.location.kind).toBe("unknown");
    expect(source.location.availability).toBe("available");
    expect(source.location.capabilities).toEqual({
      canBrowse: true,
      canReadMetadata: false,
      canPreview: false,
      canWatch: false,
      canRequestMaterialization: false,
      canAddToLibrary: false
    });
    const ephemeralAdmission = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["locationBrowse"]>>>(
      "file_workspace_location_browse",
      { request: { location: source.location.ref } }
    );
    expect(ephemeralAdmission.sessionId).not.toBe(source.sessionId);
    expect(ephemeralAdmission.location.ref).not.toEqual(source.location.ref);
    expect(ephemeralAdmission.rootPathRef).not.toEqual(source.rootPathRef);
    expect(ephemeralAdmission.location.displayName).toBe("Source");
    expect(ephemeralAdmission.location.capabilities.canBrowse).toBe(true);
    expect(ephemeralAdmission.location.capabilities.canReadMetadata).toBe(false);

    await expect(mockFileWorkspaceInvoke(
      "file_workspace_location_browse",
      { request: { location: { kind: "managed", scanRootId: "unknown-root" } } }
    )).rejects.toThrow("workspace_location_ref_unknown");
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_location_browse",
      {
        request: {
          location: {
            ...source.location.ref,
            locationId: "forged-location"
          }
        }
      }
    )).rejects.toThrow("workspace_location_ref_mismatch");
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_location_browse",
      { request: { location: source.location.ref, displayPath: "C:/renderer-path" } }
    )).rejects.toThrow("workspace_location_request_invalid");

    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: source.sessionId } });
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_location_browse",
      { request: { location: source.location.ref } }
    )).rejects.toThrow("workspace_location_ref_stale");
    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: managedAdmission.sessionId } });
    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: ephemeralAdmission.sessionId } });
  });

  it("re-resolves restore routing into fresh opaque refs and rejects stale cursors", async () => {
    const first = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>(
      "file_workspace_browse_open",
      { request: { platform: "windows", routingHint: "C:/Documents" } }
    );
    const firstPage = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: first.sessionId, requestId: "r1", pathRef: first.rootPathRef, pageSize: 1 } }
    );
    const second = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>(
      "file_workspace_browse_restore",
      {
        request: {
          locator: {
            kind: "browse",
            platform: "windows",
            routingHint: "C:/Documents"
          }
        }
      }
    );

    expect(second.sessionId).not.toBe(first.sessionId);
    expect(JSON.stringify(second)).not.toContain("C:/Documents");
    await mockFileWorkspaceInvoke(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: first.sessionId, requestId: "r2", pathRef: first.rootPathRef, pageSize: 1 } }
    );
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_browse_next_page",
      { request: { sessionId: first.sessionId, cursor: firstPage.nextCursor, pageSize: 1 } }
    )).rejects.toThrow("browse_cursor_invalid");

    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: first.sessionId } });
    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: second.sessionId } });
  });

  it("does not publish an exact knownCount for a partial page", async () => {
    const opened = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>(
      "file_workspace_browse_open",
      { request: { platform: "windows", routingHint: "C:/Documents" } }
    );
    const partial = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: opened.sessionId, requestId: "parity", pathRef: opened.rootPathRef, pageSize: 1 } }
    );
    expect(partial.completion).toBe("partial");
    expect(partial.knownCount).toBeUndefined();

    let complete = partial;
    while (complete.nextCursor !== undefined) {
      complete = await mockFileWorkspaceInvoke<BrowsePage>(
        "file_workspace_browse_next_page",
        { request: { sessionId: opened.sessionId, cursor: complete.nextCursor, pageSize: 1 } }
      );
    }
    expect(complete.completion).toBe("complete");
    expect(complete.knownCount).toBe(4);
    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: opened.sessionId } });
  });

  it("filters the mock Browse stream by query and exposes a late sentinel", async () => {
    const opened = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>(
      "file_workspace_browse_open",
      { request: { platform: "windows", routingHint: "C:/query-wire" } }
    );
    const page = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      {
        request: {
          sessionId: opened.sessionId,
          requestId: "query-wire",
          pathRef: opened.rootPathRef,
          pageSize: 1,
          query: { text: "notes", entryKind: "file" }
        }
      }
    );
    expect(page.entries.map((entry) => entry.name)).toEqual(["notes.md"]);
    expect(page.completion).toBe("complete");
    expect(page.knownCount).toBe(1);
    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: opened.sessionId } });
  });

  it("rejects Browse cancellation with missing, duplicate, or empty identity", async () => {
    const opened = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>(
      "file_workspace_browse_open",
      { request: { platform: "windows", routingHint: "C:/cancel-wire" } }
    );
    const page = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: opened.sessionId, requestId: "cancel-wire", pathRef: opened.rootPathRef, pageSize: 1 } }
    );

    await expect(mockFileWorkspaceInvoke(
      "file_workspace_browse_cancel_enumeration",
      { request: { sessionId: opened.sessionId } }
    )).rejects.toThrow("browse_cancel_requires_exactly_one_identity");
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_browse_cancel_enumeration",
      {
        request: {
          sessionId: opened.sessionId,
          requestId: "cancel-wire",
          enumeration: {
            sessionId: page.sessionId,
            requestId: page.requestId,
            enumerationId: page.enumerationId
          }
        }
      }
    )).rejects.toThrow("browse_cancel_requires_exactly_one_identity");
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_browse_cancel_enumeration",
      { request: { sessionId: opened.sessionId, requestId: "" } }
    )).rejects.toThrow("browse_cancel_requires_exactly_one_identity");

    await mockFileWorkspaceInvoke(
      "file_workspace_browse_cancel_enumeration",
      {
        request: {
          sessionId: opened.sessionId,
          enumeration: {
            sessionId: page.sessionId,
            requestId: page.requestId,
            enumerationId: page.enumerationId
          }
        }
      }
    );
    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: opened.sessionId } });
  });

  it("exposes only metadata fallback for Preview and explicitly rejects Thumbnail", async () => {
    const source = { kind: "managed" as const, fileId: "file-1" };
    const preview = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["previewCreate"]>>>(
      "file_workspace_preview_create",
      { request: { requestId: "preview-1", source, hostKind: "zen_floating" } }
    );
    const started = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["previewStart"]>>>(
      "file_workspace_preview_start",
      { request: { previewId: preview.previewId } }
    );

    expect(started.state).toBe("ready");
    expect(started.representation?.representation.family).toBe("metadata");
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_thumbnail_request",
      {
        request: {
          requestId: "thumb-1",
          source,
          variant: "small",
          workClass: "interactive"
        }
      }
    )).rejects.toThrow("thumbnail_renderer_unsupported_browser_mock");

    await mockFileWorkspaceInvoke("file_workspace_preview_dispose", { request: { previewId: preview.previewId } });
  });

  it("requires a live Browse entry before the mock reaches renderer capability", async () => {
    const opened = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>(
      "file_workspace_browse_open",
      { request: { platform: "windows", routingHint: "C:/thumbnail-identity" } }
    );
    const page = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: opened.sessionId, requestId: "identity-1", pathRef: opened.rootPathRef, pageSize: 1 } }
    );
    const entry = page.entries[0];
    expect(entry).toBeDefined();

    const request = {
      requestId: "thumb-live",
      source: entry!.ref,
      variant: "small" as const,
      workClass: "interactive" as const,
      sessionId: opened.sessionId
    };
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_thumbnail_request",
      { request }
    )).rejects.toThrow("thumbnail_renderer_unsupported_browser_mock");

    await expect(mockFileWorkspaceInvoke(
      "file_workspace_thumbnail_request",
      { request: { ...request, sourceGeneration: page.enumerationId } }
    )).rejects.toThrow("thumbnail_request_invalid");

    await mockFileWorkspaceInvoke(
      "file_workspace_browse_release_page",
      { request: { page } }
    );
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_thumbnail_request",
      { request: { ...request, requestId: "thumb-released" } }
    )).rejects.toThrow("thumbnail_source_unavailable");

    const superseded = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: opened.sessionId, requestId: "identity-2", pathRef: opened.rootPathRef, pageSize: 1 } }
    );
    const fresh = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: opened.sessionId, requestId: "identity-3", pathRef: opened.rootPathRef, pageSize: 1 } }
    );
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_thumbnail_request",
      {
        request: {
          ...request,
          requestId: "thumb-superseded",
          source: superseded.entries[0]!.ref
        }
      }
    )).rejects.toThrow("thumbnail_source_unavailable");

    await mockFileWorkspaceInvoke(
      "file_workspace_browse_cancel_enumeration",
      {
        request: {
          sessionId: opened.sessionId,
          enumeration: {
            sessionId: fresh.sessionId,
            requestId: fresh.requestId,
            enumerationId: fresh.enumerationId
          }
        }
      }
    );
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_thumbnail_request",
      {
        request: {
          ...request,
          requestId: "thumb-cancelled",
          source: fresh.entries[0]!.ref
        }
      }
    )).rejects.toThrow("thumbnail_source_unavailable");

    const other = await mockFileWorkspaceInvoke<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>(
      "file_workspace_browse_open",
      { request: { platform: "windows", routingHint: "C:/thumbnail-cross-session" } }
    );
    const otherPage = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: other.sessionId, requestId: "identity-other", pathRef: other.rootPathRef, pageSize: 1 } }
    );
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_thumbnail_request",
      {
        request: {
          ...request,
          requestId: "thumb-cross-session",
          source: otherPage.entries[0]!.ref
        }
      }
    )).rejects.toThrow("thumbnail_request_invalid");

    const pathPage = await mockFileWorkspaceInvoke<BrowsePage>(
      "file_workspace_browse_start_enumeration",
      { request: { sessionId: other.sessionId, requestId: "identity-path", pathRef: other.rootPathRef, pageSize: 2 } }
    );
    const nestedPath = pathPage.entries[1]!.pathRef!;
    await mockFileWorkspaceInvoke(
      "file_workspace_browse_retain_path",
      { request: { sessionId: other.sessionId, pathRef: nestedPath } }
    );
    await mockFileWorkspaceInvoke(
      "file_workspace_browse_release_page",
      { request: { page: pathPage } }
    );
    await mockFileWorkspaceInvoke(
      "file_workspace_browse_release_path",
      { request: { sessionId: other.sessionId, pathRef: nestedPath } }
    );
    await expect(mockFileWorkspaceInvoke(
      "file_workspace_browse_release_path",
      { request: { sessionId: other.sessionId, pathRef: nestedPath } }
    )).rejects.toThrow("browse_path_ref_invalid");

    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: opened.sessionId } });
    await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: other.sessionId } });
  });
});

describe("Global Search parent Browse admission", () => {
  const intent = { platform: "windows" as const, routingHint: "", globalEntryId: "global-index-id" };
  it("admits from a suspended workspace without turning a global ID into managed selection or a restore path", async () => {
    const api = fakeApi();
    const admitted = await api.browseOpen(intent);
    const initialPage: BrowsePage = { sessionId: admitted.sessionId, requestId: "search", enumerationId: "fresh", entries: [], completion: "complete" };
    const open = vi.fn(async () => ({ ...admitted, initialPage }));
    const controller = new FileWorkspaceController(fakeApi({ browseOpen: open }));
    await controller.suspend();
    expect(await controller.openBrowse(intent)).toMatchObject({ sessionId: "session" });
    expect(open).toHaveBeenCalledWith(intent);
    expect(controller.getState()).toMatchObject({ suspended: false, page: initialPage });
    expect(controller.getState().session.currentTarget?.kind).toBe("browse");
    await controller.dispose();
  });
  it("does not overtake navigation while the pre-admission guard is pending", async () => {
    const guard = deferred<boolean>();
    const open = vi.fn(fakeApi().browseOpen);
    const controller = new FileWorkspaceController(fakeApi({ browseOpen: open }));
    const pending = controller.openBrowse(intent, () => guard.promise);
    controller.navigate({ kind: "library", source: "custom", key: "newer-navigation" });
    guard.resolve(true);
    expect(await pending).toBeNull();
    expect(open).not.toHaveBeenCalled();
    expect(controller.getState().session.currentTarget).toMatchObject({ key: "newer-navigation" });
    await controller.dispose();
  });
  it("keeps the complete current workspace unchanged on source/admission rejection", async () => {
    const controller = new FileWorkspaceController(fakeApi({ browseOpen: async () => { throw new Error("browse_directory_permission_denied"); } }));
    controller.navigate({ kind: "library", source: "custom", key: "existing" });
    await controller.suspend();
    const before = controller.getState();
    await expect(controller.openBrowse(intent)).rejects.toThrow("browse_directory_permission_denied");
    expect(controller.getState()).toEqual(before);
    await controller.dispose();
  });
  it.each(["expired", "query-changed"])("disposes a delayed %s admission without publishing", async () => {
    const admission = deferred<Awaited<ReturnType<FileWorkspaceApi["browseOpen"]>>>();
    const dispose = vi.fn(async () => undefined);
    const controller = new FileWorkspaceController(fakeApi({ browseOpen: () => admission.promise, browseDispose: dispose }));
    controller.navigate({ kind: "library", source: "custom", key: "existing" });
    let accepted = true;
    const pending = controller.openBrowse(intent, () => accepted);
    await Promise.resolve();
    const before = controller.getState();
    accepted = false;
    admission.resolve(await fakeApi().browseOpen(intent));
    expect(await pending).toBeNull();
    expect(controller.getState()).toEqual(before);
    expect(dispose).toHaveBeenCalledWith({ sessionId: "session" });
    await controller.dispose();
  });
});
