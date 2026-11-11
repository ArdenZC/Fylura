// @vitest-environment happy-dom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FileWorkspaceApi } from "../src/api/fileWorkspaceApi";
import { fileManagementApi, type KnownLocation } from "../src/api/fileManagementApi";
import { FileWorkspaceController, WorkspaceSession } from "../src/fileWorkspace";
import { makeTranslator } from "../src/i18n";
import { mockFileWorkspaceInvoke } from "../src/api/fileWorkspaceMockApi";
import { mountedLocations } from "../src/views/fileLibrary/navigation/mountedLocations";
import { projectKnownLocations, projectMountedNetworkStatuses } from "../src/views/fileLibrary/browse/useBrowsePlaces";
import { FileLibraryExperienceProvider, useFileLibraryExperience } from "../src/views/fileLibrary/FileLibraryExperienceProvider";
import {
  FileLibraryExperienceController,
  LEGACY_LIBRARY_MIGRATION_TARGET
} from "../src/views/fileLibrary/fileLibraryExperience";
import { adaptBrowsePageCollection } from "../src/views/fileLibrary/presentation/adapters";
import {
  appendBrowseBreadcrumb,
  browseBreadcrumbChainForPath,
  browseBreadcrumbKey,
  browseEnumerationStateForPage,
  createBrowseBreadcrumb,
  isActivatableLocation,
  locationAvailabilityLabel,
  locationRefSessionId,
  mergeBrowseEntries,
  useBrowseSourceOwner,
  type BrowseSourceOwner
} from "../src/views/fileLibrary/browse/browseSourceOwner";
import { BrowseLocationPicker } from "../src/views/fileLibrary/browse/BrowseMode";
import type { BrowseEntry, BrowseOpenResponse, BrowsePage, LocationDescriptor } from "../src/types/fileWorkspace";

const dialogMocks = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);

const t = makeTranslator("en");

let mountedRoot: Root | null = null;
let mountedContainer: HTMLDivElement | null = null;

function browseResponse(sessionId: string, canWatch: boolean): BrowseOpenResponse {
  return {
    sessionId,
    location: {
      ref: { kind: "ephemeral", browseSessionId: sessionId, locationId: `location-${sessionId}` },
      displayName: "Fixture root",
      kind: "local",
      availability: "available",
      freshness: "current",
      capabilities: {
        canBrowse: true,
        canReadMetadata: true,
        canPreview: false,
        canWatch,
        canRequestMaterialization: false,
        canAddToLibrary: false
      }
    },
    rootPathRef: { id: `root-${sessionId}` }
  };
}

function networkBrowseResponse(sessionId: string, canWatch = false): BrowseOpenResponse {
  const response = browseResponse(sessionId, canWatch);
  response.location.kind = "network";
  return response;
}

function knownNetworkLocation(): KnownLocation {
  return {
    id: "volume:\\\\nas.example\\documents",
    kind: "volume",
    path: "\\\\nas.example\\documents",
    displayName: "NAS documents",
    available: false,
    volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
  };
}

function pageFor(sessionId: string, requestId: string, marker: string): BrowsePage {
  return {
    sessionId,
    requestId,
    enumerationId: `enumeration-${marker}`,
    entries: [{
      ref: { kind: "ephemeral", browseSessionId: sessionId, entryId: `entry-${marker}` },
      name: marker,
      displayPath: marker,
      kind: "file",
      materialization: "unknown"
    }],
    completion: "complete",
    knownCount: 1
  };
}

function pageForEntries(sessionId: string, requestId: string, entries: BrowseEntry[], marker: string): BrowsePage {
  return {
    sessionId,
    requestId,
    enumerationId: `enumeration-${marker}`,
    entries,
    completion: "complete",
    knownCount: entries.length
  };
}

function sourceOwnerApi(options: {
  canWatch?: boolean;
  responses?: BrowseOpenResponse[];
  browseOpen?: FileWorkspaceApi["browseOpen"];
  browseStartEnumeration?: FileWorkspaceApi["browseStartEnumeration"];
  browseNextPage?: FileWorkspaceApi["browseNextPage"];
  initialEntries?: BrowseEntry[];
  changePending?: FileWorkspaceApi["changePending"];
  changeRefresh?: FileWorkspaceApi["changeRefresh"];
} = {}) {
  const canWatch = options.canWatch ?? true;
  const responses = options.responses ?? [browseResponse("session-1", canWatch)];
  const initialEntries = options.initialEntries;
  let responseIndex = 0;
  const browseOpen = options.browseOpen ?? vi.fn(async () => responses[Math.min(responseIndex++, responses.length - 1)]!);
  let enumerationSequence = 0;
  const defaultBrowseStartEnumeration: FileWorkspaceApi["browseStartEnumeration"] = async ({ sessionId, requestId }) => {
    enumerationSequence += 1;
    return initialEntries === undefined
      ? pageFor(sessionId, requestId, `start-${sessionId}-${enumerationSequence}`)
      : pageForEntries(sessionId, requestId, initialEntries, `start-${sessionId}-${enumerationSequence}`);
  };
  const browseStartEnumeration = vi.fn(options.browseStartEnumeration ?? defaultBrowseStartEnumeration);
  const browseNextPage = vi.fn(options.browseNextPage ?? (async ({ sessionId, cursor }) =>
    pageFor(sessionId, cursor, `next-${cursor}`)));
  const monitorSessions = new Map<string, string>();
  const changeStart = vi.fn(async ({ sessionId, pathRef }: Parameters<FileWorkspaceApi["changeStart"]>[0]) => {
    const monitorId = `monitor-${sessionId}-${pathRef.id}`;
    monitorSessions.set(monitorId, sessionId);
    return { monitorId, sessionId, pathRef };
  });
  const changePending = options.changePending ?? vi.fn(async () => null);
  const changeRefresh = options.changeRefresh ?? vi.fn(async ({ monitorId }: Parameters<FileWorkspaceApi["changeRefresh"]>[0]) => {
    const sessionId = monitorSessions.get(monitorId) ?? "session-1";
    return pageFor(sessionId, `refresh-${monitorId}`, `change-${monitorId}`);
  });
  const api: FileWorkspaceApi = {
    browseOpen,
    browseRestore: async () => { throw new Error("restore not used"); },
    locationBrowse: async () => { throw new Error("location browse not used"); },
    browseStartEnumeration,
    browseNextPage,
    browseCancel: async () => undefined,
    browseReleasePage: async () => undefined,
    browseReleasePath: async () => undefined,
    browseRetainPath: async () => undefined,
    browseDispose: async () => undefined,
    locationList: async () => [],
    changeStart,
    changePending,
    changeRefresh,
    changeDispose: async () => undefined,
    readEligibility: async ({ source }) => ({ source, eligibility: "eligible" }),
    thumbnailRequest: async () => ({ cacheKey: "fixture", bytes: new Uint8Array() }),
    thumbnailCancel: async () => true,
    previewCreate: async () => { throw new Error("preview not used"); },
    previewSnapshot: async () => { throw new Error("preview not used"); },
    previewStart: async () => { throw new Error("preview not used"); },
    previewCancel: async () => true,
    previewDispose: async () => true,
    previewSwitchSource: async () => { throw new Error("preview not used"); },
    previewAssetRequest: async () => { throw new Error("preview asset not used"); }
  };
  return { api, browseOpen, browseStartEnumeration, browseNextPage, changeStart, changePending, changeRefresh };
}

async function settleSourceOwner() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await new Promise<void>((resolvePromise) => setTimeout(resolvePromise, 0));
    await Promise.resolve();
  });
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

async function mountSourceOwner(api: FileWorkspaceApi, platform: NodeJS.Platform | "browser" = "win32") {
  const workspace = new FileWorkspaceController(
    api,
    new WorkspaceSession({ initialTarget: LEGACY_LIBRARY_MIGRATION_TARGET })
  );
  const experience = new FileLibraryExperienceController(workspace);
  let latestSource: BrowseSourceOwner | null = null;
  function Probe() {
    const { controller, state } = useFileLibraryExperience();
    const source = useBrowseSourceOwner({ controller, state, t, platform });
    latestSource = source;
    return createElement("output", { "data-w204-change-state": source.changeState });
  }
  mountedContainer = document.createElement("div");
  document.body.appendChild(mountedContainer);
  mountedRoot = createRoot(mountedContainer);
  await act(async () => {
    mountedRoot!.render(createElement(
      FileLibraryExperienceProvider,
      { active: true, controller: experience, children: createElement(Probe) }
    ));
  });
  await settleSourceOwner();
  return {
    experience,
    source: () => {
      if (latestSource === null) throw new Error("Browse source owner has not rendered");
      return latestSource;
    }
  };
}

afterEach(async () => {
  if (mountedRoot !== null) {
    const root = mountedRoot;
    mountedRoot = null;
    await act(async () => {
      root.unmount();
      await new Promise<void>((resolvePromise) => setTimeout(resolvePromise, 0));
    });
  }
  mountedContainer?.remove();
  mountedContainer = null;
  document.body.innerHTML = "";
  dialogMocks.open.mockReset();
  vi.restoreAllMocks();
});

function browseEntry(overrides: Partial<BrowseEntry> = {}): BrowseEntry {
  return {
    ref: { kind: "ephemeral", browseSessionId: "session-1", entryId: "entry-1" },
    pathRef: { id: "path-1" },
    name: "Folder",
    displayPath: "Folder",
    kind: "directory",
    materialization: "unknown",
    ...overrides
  };
}

function location(availability: LocationDescriptor["availability"], canBrowse: boolean): LocationDescriptor {
  return {
    ref: { kind: "managed", scanRootId: `root-${availability}` },
    displayName: "Location",
    kind: "local",
    availability,
    freshness: "current",
    capabilities: {
      canBrowse,
      canReadMetadata: false,
      canPreview: false,
      canWatch: false,
      canRequestMaterialization: false,
      canAddToLibrary: false
    }
  };
}

describe("W2-04 Browse source owner contracts", () => {
  it("deduplicates within the live session and preserves opaque directory refs", () => {
    const first = browseEntry();
    const duplicate = browseEntry({ name: "Updated display name" });
    const merged = mergeBrowseEntries([], [first, duplicate]);

    expect(merged).toHaveLength(1);
    expect(merged[0]?.displayName).toBe("Folder");
    expect(merged[0]?.entryRef).toEqual(first.ref);
    expect(merged[0]?.pathRef).toEqual(first.pathRef);
    expect(locationRefSessionId({ kind: "ephemeral", browseSessionId: "session-1", locationId: "location-1" })).toBe("session-1");
    expect(locationRefSessionId({ kind: "managed", scanRootId: "root-1" })).toBeNull();
  });

  it("publishes exact collection count only for complete Browse truth", () => {
    const partial: BrowsePage = {
      sessionId: "session-1",
      requestId: "request-1",
      enumerationId: "enumeration-1",
      entries: [browseEntry()],
      nextCursor: "cursor-1",
      completion: "partial"
    };
    const complete: BrowsePage = {
      ...partial,
      requestId: "request-2",
      enumerationId: "enumeration-2",
      nextCursor: undefined,
      completion: "complete",
      knownCount: 1
    };

    expect(adaptBrowsePageCollection(partial).provenance).toEqual({
      sessionId: "session-1",
      requestId: "request-1",
      enumerationId: "enumeration-1",
      completion: "partial"
    });
    expect(adaptBrowsePageCollection(complete).provenance.knownCount).toBe(1);
    expect(browseEnumerationStateForPage({ completion: "partial" })).toBe("partial");
    expect(browseEnumerationStateForPage({ completion: "complete" })).toBe("complete");
  });

  it("projects breadcrumb ancestry by path identity, including branch navigation", () => {
    const sessionId = "session-1";
    const root = createBrowseBreadcrumb(sessionId, { id: "root" }, "Root");
    const a = createBrowseBreadcrumb(sessionId, { id: "a" }, "A");
    const b = createBrowseBreadcrumb(sessionId, { id: "b" }, "B");
    const c = createBrowseBreadcrumb(sessionId, { id: "c" }, "C");
    const chains = new Map<string, readonly ReturnType<typeof createBrowseBreadcrumb>[]>();
    chains.set(browseBreadcrumbKey(sessionId, root.pathRef), [root]);
    chains.set(browseBreadcrumbKey(sessionId, a.pathRef), appendBrowseBreadcrumb([root], a));
    chains.set(browseBreadcrumbKey(sessionId, b.pathRef), appendBrowseBreadcrumb([root, a], b));

    const branchFromA = appendBrowseBreadcrumb(
      browseBreadcrumbChainForPath(chains, sessionId, a.pathRef)!,
      c
    );
    chains.set(browseBreadcrumbKey(sessionId, c.pathRef), branchFromA);

    expect(browseBreadcrumbChainForPath(chains, sessionId, b.pathRef)?.map((item) => item.label)).toEqual([
      "Root", "A", "B"
    ]);
    expect(browseBreadcrumbChainForPath(chains, sessionId, a.pathRef)?.map((item) => item.label)).toEqual([
      "Root", "A"
    ]);
    expect(browseBreadcrumbChainForPath(chains, sessionId, c.pathRef)?.map((item) => item.label)).toEqual([
      "Root", "A", "C"
    ]);
    expect(browseBreadcrumbChainForPath(chains, "session-2", a.pathRef)).toBeNull();
    expect(browseBreadcrumbChainForPath(chains, sessionId, { id: "stale" })).toBeNull();
  });

  it("starts no monitor for a non-watchable target and falls back to ordinary enumeration without polling", async () => {
    const fixture = sourceOwnerApi({ canWatch: false });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.openBrowse({ platform: "windows", routingHint: "fixture" });
    });
    await settleSourceOwner();

    expect(fixture.changeStart).not.toHaveBeenCalled();
    expect(fixture.changePending).not.toHaveBeenCalled();
    expect(mounted.source().changeState).toBe("unavailable");

    await act(async () => {
      await mounted.source().refreshEnumeration();
    });
    expect(fixture.changePending).not.toHaveBeenCalled();
    expect(fixture.browseStartEnumeration).toHaveBeenCalledTimes(2);
  });

  it("feeds explicit network root enumeration failures and success back to the mounted status, including a later reopen", async () => {
    const firstResponse = browseResponse("network-session-1", false);
    firstResponse.location.kind = "unknown";
    const secondResponse = browseResponse("network-session-2", false);
    secondResponse.location.kind = "unknown";
    const enumerate = vi.fn(async ({ sessionId, requestId }: Parameters<FileWorkspaceApi["browseStartEnumeration"]>[0]) => {
      if (sessionId === "network-session-1") throw new Error("browse_directory_offline");
      return pageFor(sessionId, requestId, "reconnected-share");
    });
    const fixture = sourceOwnerApi({ responses: [firstResponse, secondResponse], browseStartEnumeration: enumerate, canWatch: false });
    const publishNetworkResult = vi.spyOn(mountedLocations, "recordNetworkBrowseSessionResult");
    const knownNetworkLocation: KnownLocation = {
      id: "volume:\\\\nas.example\\documents",
      kind: "volume",
      path: "\\\\nas.example\\documents",
      displayName: "NAS documents",
      available: false,
      volume: { identity: null, kind: "network", readOnly: null, availableBytes: null, totalBytes: null, canEject: false, availability: "connected_unverified" }
    };
    vi.spyOn(fileManagementApi, "fileLibraryKnownLocations").mockResolvedValue([knownNetworkLocation]);
    await mountedLocations.refresh(true);
    const mounted = await mountSourceOwner(fixture.api);
    const locationId = knownNetworkLocation.id;
    const path = knownNetworkLocation.path;

    let firstOpen!: { opened: boolean; sessionId?: string };
    await act(async () => {
      firstOpen = await mounted.source().openKnownLocation(path, "NAS documents") as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(locationId, path, "connected_unverified", firstOpen.sessionId);
    });
    await settleSourceOwner();
    expect(firstOpen).toMatchObject({ opened: true, sessionId: "network-session-1" });
    expect(publishNetworkResult).toHaveBeenCalledWith("network-session-1", "offline");
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "offline" } });

    await mountedLocations.refresh(true);
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "connected_unverified" } });

    let secondOpen!: { opened: boolean; sessionId?: string };
    await act(async () => {
      secondOpen = await mounted.source().openKnownLocation(path, "NAS documents") as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(locationId, path, "connected_unverified", secondOpen.sessionId);
    });
    await settleSourceOwner();
    expect(secondOpen).toMatchObject({ opened: true, sessionId: "network-session-2" });
    expect(publishNetworkResult).toHaveBeenLastCalledWith("network-session-2", "available");
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
    expect(enumerate).toHaveBeenCalledTimes(2);
  });

  it("updates the shared SMB state after a child disconnect and verifies a later reopen", async () => {
    const smb = knownNetworkLocation();
    vi.spyOn(fileManagementApi, "fileLibraryKnownLocations").mockResolvedValue([smb]);
    await mountedLocations.refresh(true);

    const firstResponse = networkBrowseResponse("network-child-session-1");
    const reopenedResponse = networkBrowseResponse("network-child-session-2");
    const childFolder = browseEntry({
      ref: { kind: "ephemeral", browseSessionId: firstResponse.sessionId, entryId: "child-folder" },
      pathRef: { id: "child-path" },
      name: "Reports",
      displayPath: "Reports"
    });
    const enumerate = vi.fn(async ({ sessionId, requestId, pathRef }: Parameters<FileWorkspaceApi["browseStartEnumeration"]>[0]) => {
      if (sessionId === firstResponse.sessionId && pathRef.id === firstResponse.rootPathRef.id) {
        return pageForEntries(sessionId, requestId, [childFolder], "network-root");
      }
      if (sessionId === firstResponse.sessionId) throw new Error("browse_directory_offline");
      return pageFor(sessionId, requestId, "network-reconnected");
    });
    const fixture = sourceOwnerApi({
      responses: [firstResponse, reopenedResponse],
      browseStartEnumeration: enumerate,
      canWatch: false
    });
    const mounted = await mountSourceOwner(fixture.api);

    let opened!: { opened: boolean; sessionId?: string };
    await act(async () => {
      opened = await mounted.source().openKnownLocation(smb.path, smb.displayName) as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", opened.sessionId);
    });
    await settleSourceOwner();
    expect(opened.sessionId).toBe(firstResponse.sessionId);
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });

    await act(async () => {
      expect(mounted.source().navigateInto(mounted.source().entries[0]!)).toBe(true);
    });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "offline" } });
    const [place] = projectKnownLocations([smb], t);
    const [projectedPlace] = projectMountedNetworkStatuses([place!], mountedLocations.getSnapshot().locations);
    expect(projectedPlace).toMatchObject({ openable: true, availability: "offline" });

    await act(async () => {
      opened = await mounted.source().openKnownLocation(smb.path, smb.displayName) as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", opened.sessionId);
    });
    await settleSourceOwner();
    expect(opened.sessionId).toBe(reopenedResponse.sessionId);
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
    const [reconnectedPlace] = projectMountedNetworkStatuses([place!], mountedLocations.getSnapshot().locations);
    expect(reconnectedPlace).toMatchObject({ openable: true, availability: "available" });
  });

  it("updates SMB status when a later page is denied, ignores an expired cursor, and recovers on a successful page", async () => {
    const smb = knownNetworkLocation();
    vi.spyOn(fileManagementApi, "fileLibraryKnownLocations").mockResolvedValue([smb]);
    await mountedLocations.refresh(true);

    const response = networkBrowseResponse("network-page-session");
    const start = vi.fn(async ({ sessionId, requestId }: Parameters<FileWorkspaceApi["browseStartEnumeration"]>[0]) => {
      const page = pageFor(sessionId, requestId, "first-network-page");
      page.completion = "partial";
      page.nextCursor = "network-cursor";
      return page;
    });
    let nextCount = 0;
    const next = vi.fn(async ({ sessionId, cursor }: Parameters<FileWorkspaceApi["browseNextPage"]>[0]) => {
      nextCount += 1;
      if (nextCount === 1) throw new Error("browse_cursor_invalid");
      if (nextCount === 2) throw new Error("browse_directory_permission_denied");
      const page = pageFor(sessionId, cursor, "network-page-recovered");
      page.enumerationId = "enumeration-first-network-page";
      return page;
    });
    const fixture = sourceOwnerApi({ responses: [response], browseStartEnumeration: start, browseNextPage: next, canWatch: false });
    const mounted = await mountSourceOwner(fixture.api);

    await act(async () => {
      const opened = await mounted.source().openKnownLocation(smb.path, smb.displayName) as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", opened.sessionId);
    });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });

    await act(async () => { await mounted.source().loadNextPage(); });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
    await act(async () => { await mounted.source().loadNextPage(); });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "permission_required" } });
    await act(async () => { await mounted.source().loadNextPage(); });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
    expect(next).toHaveBeenCalledTimes(3);
  });

  it("propagates subdirectory change-refresh disconnects and restores availability after a successful refresh", async () => {
    const smb = knownNetworkLocation();
    vi.spyOn(fileManagementApi, "fileLibraryKnownLocations").mockResolvedValue([smb]);
    await mountedLocations.refresh(true);

    const response = networkBrowseResponse("network-change-session", true);
    const childFolder = browseEntry({
      ref: { kind: "ephemeral", browseSessionId: response.sessionId, entryId: "change-child" },
      pathRef: { id: "change-child-path" }
    });
    const start = vi.fn(async ({ sessionId, requestId, pathRef }: Parameters<FileWorkspaceApi["browseStartEnumeration"]>[0]) =>
      pathRef.id === response.rootPathRef.id
        ? pageForEntries(sessionId, requestId, [childFolder], "change-root")
        : pageFor(sessionId, requestId, "change-child-page"));
    const pending = vi.fn(async ({ monitorId }: Parameters<FileWorkspaceApi["changePending"]>[0]) => ({
      monitorId,
      sequence: 1,
      hint: { kind: "content_changed" as const }
    }));
    let refreshCount = 0;
    const refresh = vi.fn(async ({ requestId }: Parameters<FileWorkspaceApi["changeRefresh"]>[0]) => {
      refreshCount += 1;
      if (refreshCount === 1) throw new Error("browse_directory_offline");
      return pageFor(response.sessionId, requestId, "change-refresh-reconnected");
    });
    const fixture = sourceOwnerApi({
      responses: [response],
      browseStartEnumeration: start,
      changePending: pending,
      changeRefresh: refresh
    });
    const mounted = await mountSourceOwner(fixture.api);

    await act(async () => {
      const opened = await mounted.source().openKnownLocation(smb.path, smb.displayName) as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", opened.sessionId);
    });
    await settleSourceOwner();
    await act(async () => {
      expect(mounted.source().navigateInto(mounted.source().entries[0]!)).toBe(true);
    });
    await settleSourceOwner();
    expect(mounted.source().changeState).toBe("watching");

    await act(async () => { await mounted.source().refreshEnumeration(); });
    await settleSourceOwner();
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "offline" } });
    await act(async () => { await mounted.source().refreshEnumeration(); });
    await settleSourceOwner();
    expect(refresh).toHaveBeenCalledTimes(2);
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
  });

  it("keeps a newer manual refresh authoritative when an older monitor read finishes late", async () => {
    const smb = knownNetworkLocation();
    vi.spyOn(fileManagementApi, "fileLibraryKnownLocations").mockResolvedValue([smb]);
    await mountedLocations.refresh(true);
    const response = networkBrowseResponse("network-refresh-race-session", true);
    const oldPending = deferred<Awaited<ReturnType<FileWorkspaceApi["changePending"]>>>();
    let pendingCount = 0;
    const pending = vi.fn(async () => {
      pendingCount += 1;
      return pendingCount === 1 ? oldPending.promise : null;
    });
    let enumerationCount = 0;
    const start = vi.fn(async ({ sessionId, requestId }: Parameters<FileWorkspaceApi["browseStartEnumeration"]>[0]) => {
      enumerationCount += 1;
      if (enumerationCount === 2) throw new Error("browse_directory_offline");
      return pageFor(sessionId, requestId, "refresh-race-page");
    });
    const fixture = sourceOwnerApi({
      responses: [response],
      browseStartEnumeration: start,
      changePending: pending
    });
    const mounted = await mountSourceOwner(fixture.api);

    await act(async () => {
      const opened = await mounted.source().openKnownLocation(smb.path, smb.displayName) as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", opened.sessionId);
    });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });

    let oldRefresh!: Promise<void>;
    await act(async () => {
      oldRefresh = mounted.source().refreshEnumeration();
      await Promise.resolve();
    });
    expect(pending).toHaveBeenCalledTimes(1);

    await act(async () => { await mounted.source().refreshEnumeration(); });
    await settleSourceOwner();
    expect(enumerationCount).toBe(2);
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "offline" } });

    oldPending.resolve({
      monitorId: "monitor-network-refresh-race-session-root-network-refresh-race-session",
      sequence: 1,
      hint: { kind: "content_changed" }
    });
    await act(async () => { await oldRefresh; });
    await settleSourceOwner();
    expect(enumerationCount).toBe(2);
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "offline" } });
  });

  it("ignores canceled and stale enumeration errors instead of changing the SMB status", async () => {
    const smb = knownNetworkLocation();
    vi.spyOn(fileManagementApi, "fileLibraryKnownLocations").mockResolvedValue([smb]);
    await mountedLocations.refresh(true);
    const response = networkBrowseResponse("network-control-error-session");
    const failures = ["browse_cancelled", "browse_enumeration_stale", "browse_directory_not_found"];
    let attempt = 0;
    const start = vi.fn(async ({ sessionId, requestId }: Parameters<FileWorkspaceApi["browseStartEnumeration"]>[0]) => {
      if (attempt === 0) {
        attempt += 1;
        return pageFor(sessionId, requestId, "control-error-initial-success");
      }
      const failure = failures[attempt - 1];
      attempt += 1;
      if (failure !== undefined) throw new Error(failure);
      return pageFor(sessionId, requestId, "control-error-success");
    });
    const fixture = sourceOwnerApi({ responses: [response], browseStartEnumeration: start, canWatch: false });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      const opened = await mounted.source().openKnownLocation(smb.path, smb.displayName) as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", opened.sessionId);
    });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });

    await act(async () => { await mounted.source().refreshEnumeration(); });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
    await act(async () => { await mounted.source().refreshEnumeration(); });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
    await act(async () => { await mounted.source().refreshEnumeration(); });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: false, volume: { availability: "unknown" } });
    await act(async () => { await mounted.source().refreshEnumeration(); });
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
  });

  it("does not let a late result from a replaced filtered enumeration alter SMB status", async () => {
    const smb = knownNetworkLocation();
    vi.spyOn(fileManagementApi, "fileLibraryKnownLocations").mockResolvedValue([smb]);
    await mountedLocations.refresh(true);

    const response = networkBrowseResponse("network-filter-session");
    const late = deferred<BrowsePage>();
    let starts = 0;
    const start = vi.fn(async ({ sessionId, requestId }: Parameters<FileWorkspaceApi["browseStartEnumeration"]>[0]) => {
      starts += 1;
      if (starts === 2) return late.promise;
      return pageFor(sessionId, requestId, "filtered-success");
    });
    const fixture = sourceOwnerApi({ responses: [response], browseStartEnumeration: start, canWatch: false });
    const mounted = await mountSourceOwner(fixture.api);

    await act(async () => {
      const opened = await mounted.source().openKnownLocation(smb.path, smb.displayName) as { opened: boolean; sessionId?: string };
      mountedLocations.recordNetworkBrowse(smb.id, smb.path, "connected_unverified", opened.sessionId);
    });
    await settleSourceOwner();
    expect(starts).toBe(1);

    await act(async () => { mounted.source().setQueryText("older-filter"); });
    await settleSourceOwner();
    expect(starts).toBe(2);
    await act(async () => { mounted.source().setQueryText("newer-filter"); });
    await settleSourceOwner();
    expect(starts).toBe(3);
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });

    late.reject(new Error("browse_directory_offline"));
    await settleSourceOwner();
    expect(mountedLocations.getSnapshot().locations[0]).toMatchObject({ available: true, volume: { availability: "available" } });
  });

  it("checks an active monitor once and refreshes through the existing change seam when a hint exists", async () => {
    const pending = vi.fn(async () => ({
      monitorId: "monitor-session-1-root-session-1",
      sequence: 1,
      hint: { kind: "content_changed" as const }
    }));
    const fixture = sourceOwnerApi({ changePending: pending });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.openBrowse({ platform: "windows", routingHint: "fixture" });
    });
    await settleSourceOwner();

    expect(fixture.changeStart).toHaveBeenCalledTimes(1);
    expect(fixture.changePending).not.toHaveBeenCalled();
    const initialEnumerations = fixture.browseStartEnumeration.mock.calls.length;

    await act(async () => {
      await mounted.source().refreshEnumeration();
    });
    await settleSourceOwner();

    expect(fixture.changePending).toHaveBeenCalledTimes(1);
    expect(fixture.changeRefresh).toHaveBeenCalledTimes(1);
    expect(fixture.browseStartEnumeration).toHaveBeenCalledTimes(initialEnumerations);
    expect(mounted.source().changeState).toBe("watching");
    expect(mounted.source().completion).toBe("complete");
  });

  it("uses ordinary enumeration when a watch monitor reports no hint", async () => {
    const fixture = sourceOwnerApi({
      changePending: vi.fn(async () => null),
    });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.openBrowse({ platform: "windows", routingHint: "fixture-a" });
    });
    await settleSourceOwner();
    expect(fixture.changeStart).toHaveBeenCalledTimes(1);

    await act(async () => {
      await mounted.source().refreshEnumeration();
    });
    await settleSourceOwner();
    expect(fixture.changePending).toHaveBeenCalledTimes(1);
    expect(fixture.changeRefresh).not.toHaveBeenCalled();
    expect(fixture.browseStartEnumeration).toHaveBeenCalledTimes(2);
    expect(mounted.source().changeState).toBe("watching");
  });

  it("ignores a stale change refresh after the Browse target switches", async () => {
    const refresh = deferred<BrowsePage>();
    const responses = [browseResponse("session-1", true), browseResponse("session-2", true)];
    const pending = vi.fn(async () => ({
      monitorId: "monitor-session-1-root-session-1",
      sequence: 2,
      hint: { kind: "uncertain" as const }
    }));
    const fixture = sourceOwnerApi({
      responses,
      changePending: pending,
      changeRefresh: vi.fn(async () => refresh.promise)
    });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.openBrowse({ platform: "windows", routingHint: "fixture-a" });
    });
    await settleSourceOwner();

    let refreshPromise!: Promise<void>;
    await act(async () => {
      refreshPromise = mounted.source().refreshEnumeration();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(fixture.changePending).toHaveBeenCalledTimes(1);
    expect(fixture.changeRefresh).toHaveBeenCalledTimes(1);

    await act(async () => {
      await mounted.experience.openBrowse({ platform: "windows", routingHint: "fixture-b" });
    });
    await settleSourceOwner();
    expect(fixture.changeStart).toHaveBeenCalledTimes(2);

    refresh.resolve(pageFor("session-1", "late-refresh", "late-old-session"));
    await act(async () => {
      await refreshPromise;
    });
    await settleSourceOwner();

    expect(mounted.source().sessionId).toBe("session-2");
    expect(mounted.source().entries.some((entry) => entry.displayName === "late-old-session")).toBe(false);
  });

  it("fails closed for unavailable location descriptors", () => {
    expect(isActivatableLocation(location("available", true))).toBe(true);
    for (const availability of ["offline", "disconnected", "permission_denied", "authentication_required", "not_found", "unknown"] as const) {
      expect(isActivatableLocation(location(availability, true))).toBe(false);
      expect(locationAvailabilityLabel(availability, t)).not.toBe("");
    }
    expect(isActivatableLocation(location("available", false))).toBe(false);
  });

  it("keeps a backend-admitted Browse location activatable after location refresh", () => {
    const admitted = location("available", true);
    expect(admitted.capabilities.canBrowse).toBe(true);
    expect(isActivatableLocation(admitted)).toBe(true);
  });

  it("clears an earlier location-list error after a later pane load succeeds", async () => {
    const fixture = sourceOwnerApi();
    const locationList = vi.fn<FileWorkspaceApi["locationList"]>();
    fixture.api.locationList = locationList;
    locationList.mockRejectedValueOnce(new Error("temporary location list failure"));
    locationList.mockResolvedValueOnce([location("available", true)]);
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => { await mounted.experience.switchMode("browse"); });
    await settleSourceOwner();
    expect(mounted.source().locationError).toBe(true);

    await act(async () => {
      await mounted.experience.workspace.loadLocations();
    });
    await settleSourceOwner();
    expect(mounted.source().locationError).toBe(false);
    expect(mounted.source().locations).toHaveLength(1);
  });

  it("renders the first-entry Choose Folder action and forwards the click", async () => {
    const chooseFolder = vi.fn(async () => true);
    const source = {
      locations: [],
      locationState: "ready",
      locationError: false,
      admissionError: false,
      admissionLoading: false,
      loadLocations: vi.fn(async () => undefined),
      chooseFolder
    } as unknown as BrowseSourceOwner;
    mountedContainer = document.createElement("div");
    document.body.appendChild(mountedContainer);
    mountedRoot = createRoot(mountedContainer);
    await act(async () => {
      mountedRoot!.render(createElement(BrowseLocationPicker, { detached: true, source, t }));
    });

    const button = mountedContainer.querySelector<HTMLButtonElement>("[data-browse-choose-folder]");
    expect(button?.textContent).toContain("Choose Folder");
    await act(async () => {
      button?.click();
    });
    expect(chooseFolder).toHaveBeenCalledTimes(1);
  });

  it("uses the existing openBrowse seam for first-entry folder admission", async () => {
    dialogMocks.open.mockResolvedValue("F:\\work\\NativeFixtureW609");
    const fixture = sourceOwnerApi();
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.switchMode("browse");
    });
    await settleSourceOwner();

    await expect(mounted.source().chooseFolder()).resolves.toBe(true);
    expect(dialogMocks.open).toHaveBeenCalledWith(expect.objectContaining({
      directory: true,
      multiple: false,
      title: "Choose a folder to browse"
    }));
    expect(fixture.browseOpen).toHaveBeenCalledTimes(1);
    expect(fixture.browseOpen).toHaveBeenCalledWith({
      platform: "windows",
      routingHint: "F:\\work\\NativeFixtureW609",
      displayHint: "NativeFixtureW609"
    });
    expect(mounted.experience.getState().detachedBrowse).toBe(false);
    expect(mounted.experience.getState().workspace.session.currentTarget?.kind).toBe("browse");
  });

  it("keeps detached Browse unchanged when the native folder dialog is canceled", async () => {
    dialogMocks.open.mockResolvedValue(null);
    const fixture = sourceOwnerApi();
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.switchMode("browse");
    });
    await settleSourceOwner();

    await expect(mounted.source().chooseFolder()).resolves.toBe(false);
    expect(fixture.browseOpen).not.toHaveBeenCalled();
    expect(mounted.experience.getState().detachedBrowse).toBe(true);
    expect(mounted.experience.getState().workspace.session.currentTarget?.kind).toBe("library");
    expect(mounted.source().admissionError).toBe(false);
  });

  it("reports admission failure without publishing a fake current target", async () => {
    dialogMocks.open.mockResolvedValue("F:\\work\\NativeFixtureW609");
    const fixture = sourceOwnerApi({
      browseOpen: vi.fn(async () => {
        throw new Error("browse admission failed");
      })
    });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.switchMode("browse");
    });
    await settleSourceOwner();

    await expect(mounted.source().chooseFolder()).resolves.toBe(false);
    await settleSourceOwner();
    expect(mounted.source().admissionError).toBe(true);
    expect(mounted.experience.getState().detachedBrowse).toBe(true);
    expect(mounted.experience.getState().workspace.session.currentTarget?.kind).toBe("library");
  });

  it("guards duplicate first-entry admissions while the existing openBrowse request is pending", async () => {
    dialogMocks.open.mockResolvedValue("F:\\work\\NativeFixtureW609");
    const pending = deferred<BrowseOpenResponse>();
    const fixture = sourceOwnerApi({ browseOpen: vi.fn(async () => pending.promise) });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.switchMode("browse");
    });
    await settleSourceOwner();

    let first!: Promise<boolean>;
    await act(async () => {
      first = mounted.source().chooseFolder();
      await Promise.resolve();
    });
    expect(mounted.source().admissionLoading).toBe(true);
    await expect(mounted.source().chooseFolder()).resolves.toBe(false);
    expect(dialogMocks.open).toHaveBeenCalledTimes(1);
    expect(fixture.browseOpen).toHaveBeenCalledTimes(1);

    pending.resolve(browseResponse("first-entry-session", true));
    await act(async () => {
      await first;
    });
    expect(mounted.experience.getState().detachedBrowse).toBe(false);
  });

  it("keeps existing backend LocationRef cards on the browseLocation route", async () => {
    const fixture = sourceOwnerApi();
    const locationBrowse = vi.fn(async () => browseResponse("location-session", true));
    fixture.api.locationBrowse = locationBrowse;
    const mounted = await mountSourceOwner(fixture.api);
    const existing = {
      ...location("available", true),
      ref: { kind: "ephemeral", browseSessionId: "existing-session", locationId: "existing-location" }
    } satisfies LocationDescriptor;

    await expect(mounted.source().activateLocation(existing)).resolves.toBe(true);
    expect(locationBrowse).toHaveBeenCalledWith({ location: existing.ref });
    expect(mounted.experience.getState().detachedBrowse).toBe(false);
    expect(dialogMocks.open).not.toHaveBeenCalled();
  });

  it("fails gracefully without opening a native dialog in browser or unsupported runtimes", async () => {
    const fixture = sourceOwnerApi();
    const mounted = await mountSourceOwner(fixture.api, "browser");
    await act(async () => {
      await mounted.experience.switchMode("browse");
    });
    await settleSourceOwner();

    await expect(mounted.source().chooseFolder()).resolves.toBe(false);
    await settleSourceOwner();
    expect(dialogMocks.open).not.toHaveBeenCalled();
    expect(fixture.browseOpen).not.toHaveBeenCalled();
    expect(mounted.source().admissionError).toBe(true);
    expect(mounted.experience.getState().detachedBrowse).toBe(true);
  });

  it("keeps the browser mock location surface split between openable and unavailable entries", async () => {
    const locations = await mockFileWorkspaceInvoke<LocationDescriptor[]>("file_workspace_location_list");
    expect(locations[0]).toMatchObject({ availability: "available", capabilities: { canBrowse: true } });
    expect(locations.some((entry) => entry.availability === "offline" && !entry.capabilities.canBrowse)).toBe(true);
  });

  it("keeps the Browse range anchor stable across repeated and reverse range operations", async () => {
    const initialEntries = ["entry-1", "entry-2", "entry-3", "entry-4"].map((entryId) => browseEntry({
      ref: { kind: "ephemeral", browseSessionId: "session-1", entryId },
      name: entryId
    }));
    const fixture = sourceOwnerApi({ initialEntries });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => {
      await mounted.experience.openBrowse({ platform: "windows", routingHint: "range-anchor" });
    });
    await settleSourceOwner();

    await act(async () => mounted.source().selectEntry("entry-2", "replace"));
    expect([...mounted.source().selectedIds]).toEqual(["entry-2"]);

    await act(async () => mounted.source().selectEntry("entry-3", "range"));
    expect([...mounted.source().selectedIds]).toEqual(["entry-2", "entry-3"]);

    await act(async () => mounted.source().selectEntry("entry-4", "range"));
    expect([...mounted.source().selectedIds]).toEqual(["entry-2", "entry-3", "entry-4"]);

    await act(async () => mounted.source().selectEntry("entry-3", "range"));
    expect([...mounted.source().selectedIds]).toEqual(["entry-2", "entry-3"]);

    await act(async () => mounted.source().selectEntry("entry-1", "range"));
    expect([...mounted.source().selectedIds]).toEqual(["entry-1", "entry-2"]);
  });

  it("keeps Browse presentation source-local and path-ref based", () => {
    const owner = readFileSync(resolve("src/views/fileLibrary/browse/browseSourceOwner.ts"), "utf8");
    const mode = readFileSync(resolve("src/views/fileLibrary/browse/BrowseMode.tsx"), "utf8");
    const list = readFileSync(resolve("src/views/fileLibrary/list/SharedFileList.tsx"), "utf8");

    expect(owner).toContain("adaptBrowseEntry");
    expect(owner).toContain("pathRef");
    expect(owner).toContain("page.completion");
    expect(owner).not.toContain("state.workspace.session.history");
    expect(owner).not.toContain('page.nextCursor === undefined ? "complete"');
    expect(owner).not.toContain("all_matching");
    expect(owner).not.toContain("displayPath.split");
    expect(mode).toContain("SharedFileList");
    expect(mode).toContain("source.navigateInto(entry)");
    expect(mode).toContain("data-browse-selection-authority");
    expect(list).toContain("data-browse-entry");
    expect(list).toContain("entry.entryRef.entryId");
    expect(list).not.toContain("displayPath.split");
  });
});

describe("Search admission Browse projection", () => {
  it.each([true, false])("uses only fresh enumeration authority and selects only with a verified ref (%s)", async (verified) => {
    const initialPage = pageFor("search-session", "search-admission", "search-target");
    const response = { ...browseResponse("search-session", false), initialPage,
      ...(verified ? { focusEntryRef: initialPage.entries[0]!.ref } : {}) };
    const fixture = sourceOwnerApi({ canWatch: false, responses: [response] });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => { await mounted.experience.openBrowse({ platform: "windows", routingHint: "", globalEntryId: "index-id" }); });
    await settleSourceOwner();
    expect(fixture.browseStartEnumeration).not.toHaveBeenCalled();
    expect(mounted.source().entries).toHaveLength(1);
    expect(mounted.source().selectedCount).toBe(verified ? 1 : 0);
    expect(mounted.source().focusedId).toBe(verified ? initialPage.entries[0]!.ref.entryId : null);
    await act(async () => { await mounted.source().refreshEnumeration(); });
    expect(fixture.browseStartEnumeration).toHaveBeenCalledOnce();
    expect(mounted.source().selectedCount).toBe(0);
  });
  it("ignores a ref outside the admitted enumeration", async () => {
    const initialPage = pageFor("search-session", "search-admission", "search-target");
    const response = { ...browseResponse("search-session", false), initialPage,
      focusEntryRef: { ...initialPage.entries[0]!.ref, browseSessionId: "other-session" } };
    const fixture = sourceOwnerApi({ canWatch: false, responses: [response] });
    const mounted = await mountSourceOwner(fixture.api);
    await act(async () => { await mounted.experience.openBrowse({ platform: "windows", routingHint: "", globalEntryId: "index-id" }); });
    await settleSourceOwner();
    expect(mounted.source().selectedCount).toBe(0);
  });
});
