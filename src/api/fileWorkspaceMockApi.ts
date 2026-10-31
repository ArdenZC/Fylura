import { managementFixturePage } from "./fileManagementMock";
import type {
  BrowseEntry,
  BrowseOpenRequest,
  BrowseOpenResponse,
  BrowsePage,
  BrowseQuerySpecV1,
  BrowseRestoreRequest,
  BrowseStartEnumerationRequest,
  ChangePendingResponse,
  ChangeStartRequest,
  ChangeStartResponse,
  LocationBrowseRequest,
  LocationDescriptor,
  LocationRef,
  PreviewAssetArtifact,
  PreviewAssetRequest,
  PreviewCreateRequest,
  PreviewHostKind,
  PreviewMetadata,
  PreviewRepresentationEnvelope,
  PreviewSnapshot,
  ReadEligibilityRequest,
  ReadEligibilityResponse,
  ThumbnailRequest
} from "../types/fileWorkspace";
import { parsePreviewSnapshot } from "./fileWorkspacePreviewWire";
import type { GlobalSearchResult } from "../types/domain";
import type { FolderSummaryPayloadV1 } from "./folderPreviewWire";

const FILE_WORKSPACE_COMMANDS = new Set([
  "file_workspace_browse_open",
  "file_workspace_browse_reveal",
  "file_workspace_browse_restore",
  "file_workspace_location_browse",
  "file_workspace_browse_start_enumeration",
  "file_workspace_browse_next_page",
  "file_workspace_browse_cancel_enumeration",
  "file_workspace_browse_release_page",
  "file_workspace_browse_release_path",
  "file_workspace_browse_retain_path",
  "file_workspace_browse_dispose",
  "file_workspace_location_list",
  "file_workspace_change_start",
  "file_workspace_change_pending",
  "file_workspace_change_refresh",
  "file_workspace_change_dispose",
  "file_workspace_read_eligibility",
  "file_workspace_thumbnail_request",
  "file_workspace_thumbnail_cancel",
  "file_workspace_preview_create",
  "file_workspace_preview_snapshot",
  "file_workspace_preview_start",
  "file_workspace_preview_cancel",
  "file_workspace_preview_dispose",
  "file_workspace_preview_switch_source",
  "file_workspace_preview_asset_request"
]);

type MockArgs = Record<string, unknown> | undefined;

interface MockEnumeration {
  requestId: string;
  enumerationId: string;
  cursor?: string;
  generation: number;
  query: BrowseQuerySpecV1;
  pathRefId: string;
  nextIndex: number;
}

interface MockEntry {
  enumerationId: string;
  pathRefId?: string;
}

interface MockBrowseSession {
  routingHint?: string;
  sessionId: string;
  location: BrowseOpenResponse["location"];
  rootPathRef: { id: string };
  generation: number;
  pathRefs: Set<string>;
  retainedPathRefs: Set<string>;
  entries: Map<string, MockEntry>;
  enumeration?: MockEnumeration;
  disposed: boolean;
}

interface MockPreviewRecord {
  snapshot: PreviewSnapshot;
  /** At most one bounded progressive snapshot is staged for the W3-07 fixture. */
  pendingSnapshots?: PreviewSnapshot[];
}

interface PendingPreviewStart {
  previewId: string;
  requestId: string;
  source: PreviewSnapshot["source"];
  snapshot: PreviewSnapshot;
  resolve: (snapshot: PreviewSnapshot) => void;
}

interface PendingPreviewAsset {
  artifact: PreviewAssetArtifact;
  resolve: (artifact: PreviewAssetArtifact) => void;
}

const sessions = new Map<string, MockBrowseSession>();
const previews = new Map<string, MockPreviewRecord>();
const pendingPreviewStarts: PendingPreviewStart[] = [];
const pendingPreviewAssets: PendingPreviewAsset[] = [];
const monitors = new Map<string, string>();
let nextId = 1;

// W2-11 uses a lazy, deterministic browser-only projection. It deliberately
// does not allocate a 100k-entry array; each page is generated from the raw
// index and the current query instead.
const W211_BROWSE_TOTAL = 100_000;
const W211_SCAN_BUDGET = 1_024;
const W211_LATE_SENTINEL_INDEX = 99_000;

// A tiny valid PNG keeps the browser-only thumbnail seam deterministic. It is
// a presentation fixture, not evidence of native renderer support.
const MOCK_THUMBNAIL_BYTES = new Uint8Array([
  137, 80, 78, 71, 13, 10, 26, 10,
  0, 0, 0, 13, 73, 72, 68, 82,
  0, 0, 0, 1, 0, 0, 0, 1, 8, 4, 0, 0, 0, 181, 28, 12, 2,
  0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 96, 0, 0, 0, 2, 0, 1, 229, 39, 212, 162,
  0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130
]);

// A tiny deterministic JPEG fixture for the browser-only image transport.
// Native decoder and bound evidence remains Rust-owned; this only exercises
// the shared renderer's opaque asset lifecycle.
const MOCK_JPEG_BYTES = Uint8Array.from(
  atob("/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////2wBDAf//////////////////////////////////////////////////////////////////////////////////////wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAX/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oADAMBAAIQAxAAAAH/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oACAEBAAEFAqf/xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oACAEDAQE/AX//xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oACAECAQE/AX//xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oACAEBAAY/Aqf/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oACAEBAAE/IV//2gAMAwEAAgADAAAAEP/EABQRAQAAAAAAAAAAAAAAAAAAABD/2gAIAQMBAT8Qf//EABQRAQAAAAAAAAAAAAAAAAAAABD/2gAIAQIBAT8Qf//EABQQAQAAAAAAAAAAAAAAAAAAABD/2gAIAQEAAT8Qf//Z"),
  (character) => character.charCodeAt(0)
);

const MOCK_MANAGED_LOCATION_REF: Extract<LocationRef, { kind: "managed" }> = {
  kind: "managed",
  scanRootId: "mock-scan-root"
};

const MOCK_MANAGED_LOCATION: LocationDescriptor = {
  ref: MOCK_MANAGED_LOCATION_REF,
  displayName: "Managed mock root",
  kind: "unknown",
  availability: "available",
  freshness: "current",
  capabilities: {
    canBrowse: true,
    canReadMetadata: false,
    canPreview: false,
    canWatch: false,
    canRequestMaterialization: false,
    canAddToLibrary: false
  }
};

const MOCK_UNAVAILABLE_LOCATION: LocationDescriptor = {
  ref: { kind: "managed", scanRootId: "mock-offline-root" },
  displayName: "Unavailable mock root",
  kind: "network",
  availability: "offline",
  freshness: "stale",
  capabilities: {
    canBrowse: false,
    canReadMetadata: false,
    canPreview: false,
    canWatch: false,
    canRequestMaterialization: false,
    canAddToLibrary: false
  }
};

const MOCK_UNMANAGED_LOCATION: LocationDescriptor = {
  ref: {
    kind: "ephemeral",
    browseSessionId: "mock-unmanaged-location",
    locationId: "mock-external-drive"
  },
  displayName: "Unmanaged mock drive",
  kind: "external",
  availability: "available",
  freshness: "not_applicable",
  capabilities: {
    canBrowse: true,
    canReadMetadata: false,
    canPreview: false,
    canWatch: false,
    canRequestMaterialization: false,
    canAddToLibrary: false
  }
};

function isW209PlatformFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w2-09-browser-fixture") === "platform";
}

function isW204SourceOwnerFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w2-04-browser-fixture") === "source-owner";
}

function isW211IntegratedFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w2-11-browser-fixture") === "integrated";
}

function isW302FixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w3-02-browser-fixture") === "preview";
}

function isW303FixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w3-03-browser-fixture") === "pinned";
}

function isW309IntegrationFixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w3-09-browser-fixture") === "integration";
}

function isW304FixtureEnabled() {
  if (typeof window === "undefined") return false;
  return isW309IntegrationFixtureEnabled()
    || new URLSearchParams(window.location.search).get("w3-04-browser-fixture") === "providers";
}

function isW305FixtureEnabled() {
  if (typeof window === "undefined") return false;
  return isW309IntegrationFixtureEnabled()
    || new URLSearchParams(window.location.search).get("w3-05-browser-fixture") === "providers";
}

function isW306FixtureEnabled() {
  if (typeof window === "undefined") return false;
  return isW309IntegrationFixtureEnabled()
    || new URLSearchParams(window.location.search).get("w3-06-browser-fixture") === "images";
}

function w309Stats() {
  if (!isW309IntegrationFixtureEnabled() || typeof window === "undefined") return null;
  const testWindow = window as Window & {
    __zcW309?: {
      terminalStarts: number;
      terminalCodes: string[];
    };
  };
  if (testWindow.__zcW309 === undefined) {
    testWindow.__zcW309 = { terminalStarts: 0, terminalCodes: [] };
  }
  return testWindow.__zcW309;
}

function isW308FixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w3-08-browser-fixture") === "archives";
}

function isW307FixtureEnabled() {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("w3-07-browser-fixture") === "folders";
}

function w304Stats() {
  if (!isW304FixtureEnabled() || typeof window === "undefined") return null;
  const testWindow = window as Window & {
    __zcW304?: {
      starts: number;
      richStarts: number;
      fallbackStarts: number;
    };
  };
  if (testWindow.__zcW304 === undefined) {
    testWindow.__zcW304 = { starts: 0, richStarts: 0, fallbackStarts: 0 };
  }
  return testWindow.__zcW304;
}

function w302Stats() {
  if (!isW302FixtureEnabled() || typeof window === "undefined") return null;
  const testWindow = window as Window & {
    __zcW302?: {
      pendingStartCount: number;
      started: number;
      resolved: number;
      switchCalls: number;
      cancelCalls: number;
      disposeCalls: number;
      lateStarts: number;
      createHostKinds: PreviewHostKind[];
      activeBackendHostKinds: Record<string, PreviewHostKind>;
      activeBackendSources: Record<string, PreviewSnapshot["source"]>;
      browseNextPageCalls: number;
      browseNextPageLengths: number[];
      resolveNext: () => void;
      resolveAll: () => void;
    };
  };
  if (testWindow.__zcW302 === undefined) {
    testWindow.__zcW302 = {
      pendingStartCount: 0,
      started: 0,
      resolved: 0,
      switchCalls: 0,
      cancelCalls: 0,
      disposeCalls: 0,
      lateStarts: 0,
      createHostKinds: [],
      activeBackendHostKinds: {},
      activeBackendSources: {},
      browseNextPageCalls: 0,
      browseNextPageLengths: [],
      resolveNext: resolveNextPreviewStart,
      resolveAll: resolveAllPreviewStarts
    };
  }
  return testWindow.__zcW302;
}

function w306Stats() {
  if (!isW306FixtureEnabled() || typeof window === "undefined") return null;
  const testWindow = window as Window & {
    __zcW306?: {
      pendingAssetCount: number;
      assetRequests: PreviewAssetRequest[];
      resolvedAssets: number;
      resolveAllAssets: () => void;
    };
  };
  if (testWindow.__zcW306 === undefined) {
    testWindow.__zcW306 = {
      pendingAssetCount: 0,
      assetRequests: [],
      resolvedAssets: 0,
      resolveAllAssets: resolveAllPreviewAssets
    };
  }
  return testWindow.__zcW306;
}

function w307Stats() {
  if (!isW307FixtureEnabled() || typeof window === "undefined") return null;
  const testWindow = window as Window & {
    __zcW307?: {
      starts: number;
      richStarts: number;
      fallbackStarts: number;
      lastSourceKey: string | null;
      lastSummary: string | null;
      snapshotCalls: number;
      snapshotProgress: number[];
      snapshotStates: string[];
    };
  };
  if (testWindow.__zcW307 === undefined) {
    testWindow.__zcW307 = {
      starts: 0,
      richStarts: 0,
      fallbackStarts: 0,
      lastSourceKey: null,
      lastSummary: null,
      snapshotCalls: 0,
      snapshotProgress: [],
      snapshotStates: []
    };
  }
  return testWindow.__zcW307;
}

function recordW307Snapshot(snapshot: PreviewSnapshot) {
  const fixture = w307Stats();
  if (fixture === null) return;
  fixture.snapshotCalls += 1;
  if (fixture.snapshotStates.length < 8) fixture.snapshotStates.push(snapshot.state);
  const representation = snapshot.representation?.representation;
  if (representation?.family !== "folder_summary") return;
  try {
    const payload = JSON.parse(representation.encodedSummary) as { progress?: { inspectedEntries?: unknown } };
    if (fixture.snapshotProgress.length < 8 && typeof payload.progress?.inspectedEntries === "number") {
      fixture.snapshotProgress.push(payload.progress.inspectedEntries);
    }
  } catch {
    // The real decoder owns malformed wire rejection; the fixture only records bounded diagnostics.
  }
}

function w211Stats() {
  if (!isW211IntegratedFixtureEnabled() || typeof window === "undefined") return null;
  const testWindow = window as Window & {
    __zcW211?: {
      browsePageCalls: number;
      browsePageLengths: number[];
      browseScanEnds: number[];
      browseQueries: string[];
      browseFirstPageSnapshots: Array<{ query: string; entries: number; completion: string; hasCursor: boolean; knownCount?: number }>;
      browseSessionsCreated: number;
      browseSessionsDisposed: number;
      thumbnailRequests: number;
      thumbnailCancels: number;
      activeThumbnailRequests: number;
      thumbnailVariants: string[];
    };
  };
  if (testWindow.__zcW211 === undefined) {
    testWindow.__zcW211 = {
      browsePageCalls: 0,
      browsePageLengths: [],
      browseScanEnds: [],
      browseQueries: [],
      browseFirstPageSnapshots: [],
      browseSessionsCreated: 0,
      browseSessionsDisposed: 0,
      thumbnailRequests: 0,
      thumbnailCancels: 0,
      activeThumbnailRequests: 0,
      thumbnailVariants: []
    };
  }
  return testWindow.__zcW211;
}

export function isFileWorkspaceMockCommand(command: string) {
  return FILE_WORKSPACE_COMMANDS.has(command);
}

export async function mockFileWorkspaceInvoke<T>(
  command: string,
  args?: MockArgs
): Promise<T> {
  const request = args?.request as Record<string, unknown> | undefined;
  switch (command) {
    case "file_workspace_browse_reveal":
      resolveManagementMockEntry(request?.entryRef as import("../types/fileWorkspace").EntryRef);
      throw new Error("browser_mock_reveal_unavailable");
    case "file_workspace_browse_open":
      return openBrowse(request as unknown as BrowseOpenRequest) as T;
    case "file_workspace_browse_restore":
      return restoreBrowse(request as unknown as BrowseRestoreRequest) as T;
    case "file_workspace_location_browse":
      return browseLocation(request as unknown as LocationBrowseRequest) as T;
    case "file_workspace_browse_start_enumeration":
      if (isW211IntegratedFixtureEnabled()
        && (request as unknown as BrowseStartEnumerationRequest).query?.text?.trim().toLowerCase() === "slow-a") {
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
      return startEnumeration(request as unknown as BrowseStartEnumerationRequest) as T;
    case "file_workspace_browse_next_page":
      return nextPage(request) as T;
    case "file_workspace_browse_cancel_enumeration":
      cancelEnumeration(request);
      return undefined as T;
    case "file_workspace_browse_release_page":
      releasePage(request);
      return undefined as T;
    case "file_workspace_browse_release_path":
      releasePath(request);
      return undefined as T;
    case "file_workspace_browse_retain_path":
      retainPath(request);
      return undefined as T;
    case "file_workspace_browse_dispose":
      disposeBrowse(request);
      return undefined as T;
    case "file_workspace_location_list":
      return (isW209PlatformFixtureEnabled()
        ? [MOCK_MANAGED_LOCATION, MOCK_UNMANAGED_LOCATION, MOCK_UNAVAILABLE_LOCATION]
        : [MOCK_MANAGED_LOCATION, MOCK_UNAVAILABLE_LOCATION]
      ).map((location) => ({
        ...location,
        ref: { ...location.ref },
        capabilities: { ...location.capabilities }
      })) as T;
    case "file_workspace_change_start":
      return startChange(request as unknown as ChangeStartRequest) as T;
    case "file_workspace_change_pending":
      return pendingChange(request) as T;
    case "file_workspace_change_refresh":
      throw new Error("ephemeral_change_refresh_not_pending");
    case "file_workspace_change_dispose":
      monitors.delete(String(request?.monitorId ?? ""));
      return undefined as T;
    case "file_workspace_read_eligibility":
      return readEligibility(request as unknown as ReadEligibilityRequest) as T;
    case "file_workspace_thumbnail_request": {
      const artifact = await thumbnailRequest(request as unknown as ThumbnailRequest);
      return encodeThumbnailIpcResponse(artifact.cacheKey, artifact.bytes) as T;
    }
    case "file_workspace_thumbnail_cancel":
      recordW206ThumbnailCancel();
      return false as T;
    case "file_workspace_preview_create":
      return createPreview(request as unknown as PreviewCreateRequest) as T;
    case "file_workspace_preview_snapshot":
      return previewSnapshot(request) as T;
    case "file_workspace_preview_start":
      return previewStart(request) as T;
    case "file_workspace_preview_cancel":
      return cancelPreview(request) as T;
    case "file_workspace_preview_dispose":
      disposePreview(request);
      return true as T;
    case "file_workspace_preview_switch_source":
      return switchPreviewSource(request) as T;
    case "file_workspace_preview_asset_request":
      return encodePreviewAssetIpcResponse(await previewAssetRequest(request as unknown as PreviewAssetRequest)) as T;
    default:
      throw new Error(`browser_mock_unknown_file_workspace_command:${command}`);
  }
}

function openBrowse(request: BrowseOpenRequest): BrowseOpenResponse {
  if (!request || typeof request.routingHint !== "string" || request.routingHint.length === 0) {
    throw new Error("browse_routing_hint_invalid");
  }
  const response = newBrowseSession(request.displayHint?.trim() || "Browse", true);
  getSession(response.sessionId).routingHint = request.routingHint.replace(/\\/g, "/");
  return response;
}

function browseLocation(request: LocationBrowseRequest): BrowseOpenResponse {
  if (!isLocationBrowseRequest(request)) {
    throw new Error("workspace_location_request_invalid");
  }

  if (request.location.kind === "managed") {
    if (request.location.scanRootId !== MOCK_MANAGED_LOCATION_REF.scanRootId) {
      throw new Error("workspace_location_ref_unknown");
    }
    return newBrowseSession(MOCK_MANAGED_LOCATION.displayName, true);
  }

  if (request.location.kind === "ephemeral"
    && MOCK_UNMANAGED_LOCATION.ref.kind === "ephemeral"
    && request.location.browseSessionId === MOCK_UNMANAGED_LOCATION.ref.browseSessionId
    && request.location.locationId === MOCK_UNMANAGED_LOCATION.ref.locationId) {
    return newBrowseSession(MOCK_UNMANAGED_LOCATION.displayName, true);
  }

  const source = sessions.get(request.location.browseSessionId);
  if (source === undefined || source.disposed) {
    throw new Error("workspace_location_ref_stale");
  }
  if (source.location.ref.kind !== "ephemeral"
    || source.location.ref.locationId !== request.location.locationId) {
    throw new Error("workspace_location_ref_mismatch");
  }
  return newBrowseSession(source.location.displayName || "Browse", true);
}

function newBrowseSession(displayName: string, admitted: boolean): BrowseOpenResponse {
  const sessionId = id("browse");
  const location = {
    ref: { kind: "ephemeral" as const, browseSessionId: sessionId, locationId: id("location") },
    displayName,
    kind: "unknown" as const,
    availability: admitted ? "available" as const : "unknown" as const,
    freshness: "not_applicable" as const,
    capabilities: locationCapabilities(admitted)
  } satisfies LocationDescriptor;
  const response: BrowseOpenResponse = {
    sessionId,
    location,
    rootPathRef: { id: id("path") }
  };
  sessions.set(sessionId, {
    sessionId,
    location,
    rootPathRef: response.rootPathRef,
    generation: 0,
    pathRefs: new Set([response.rootPathRef.id]),
    retainedPathRefs: new Set([response.rootPathRef.id]),
    entries: new Map(),
    disposed: false
  });
  const stats = w211Stats();
  if (stats) stats.browseSessionsCreated += 1;
  return response;
}

function locationCapabilities(canBrowse: boolean) {
  return {
    canBrowse,
    canReadMetadata: false,
    canPreview: false,
    canWatch: false,
    canRequestMaterialization: false,
    canAddToLibrary: false
  };
}

function isLocationBrowseRequest(request: unknown): request is LocationBrowseRequest {
  if (!request || typeof request !== "object") return false;
  const record = request as Record<string, unknown>;
  if (Object.keys(record).length !== 1 || !("location" in record)) return false;
  const location = record.location;
  if (!location || typeof location !== "object") return false;
  const value = location as Record<string, unknown>;
  if (value.kind === "managed") {
    return Object.keys(value).every((key) => key === "kind" || key === "scanRootId")
      && isOpaqueId(value.scanRootId)
      && !looksLikePath(value.scanRootId);
  }
  if (value.kind === "ephemeral") {
    return Object.keys(value).every((key) =>
      key === "kind" || key === "browseSessionId" || key === "locationId")
      && isOpaqueId(value.browseSessionId)
      && isOpaqueId(value.locationId)
      && !looksLikePath(value.browseSessionId)
      && !looksLikePath(value.locationId);
  }
  return false;
}

function restoreBrowse(request: BrowseRestoreRequest): BrowseOpenResponse {
  if (request.locator.kind !== "browse") throw new Error("workspace_restore_requires_browse_locator");
  return openBrowse({
    platform: request.locator.platform,
    routingHint: request.locator.routingHint,
    ...(request.locator.displayHint === undefined ? {} : { displayHint: request.locator.displayHint })
  });
}

function startEnumeration(request: BrowseStartEnumerationRequest): BrowsePage {
  const session = getSession(request.sessionId);
  if (!session.pathRefs.has(request.pathRef.id)) throw new Error("browse_path_ref_invalid");
  if (session.enumeration !== undefined) {
    invalidateEntriesForEnumeration(session, session.enumeration.enumerationId);
  }
  session.generation += 1;
  const enumerationId = `${session.sessionId}-enumeration-${session.generation}`;
  const cursor = `${enumerationId}-cursor`;
  session.enumeration = {
    requestId: request.requestId,
    enumerationId,
    cursor,
    generation: session.generation,
    query: request.query ?? { text: null, entryKind: "all" },
    pathRefId: request.pathRef.id,
    nextIndex: 0
  };
  return makePage(session, request.requestId, enumerationId, request.pageSize, session.enumeration.query);
}

function nextPage(request: MockArgs): BrowsePage {
  const session = getSession(String(request?.sessionId ?? ""));
  const enumeration = session.enumeration;
  if (!enumeration || request?.cursor !== enumeration.cursor) throw new Error("browse_cursor_invalid");
  const page = makePage(session, enumeration.requestId, enumeration.enumerationId, Number(request?.pageSize ?? 1), enumeration.query);
  const fixture = w302Stats();
  if (fixture !== null && isW303QueryGap(enumeration.query)) {
    fixture.browseNextPageCalls += 1;
    fixture.browseNextPageLengths.push(page.entries.length);
  }
  return page;
}

function sortedMockEntries(entries: BrowseEntry[], query: BrowseQuerySpecV1) {
  const sort = query.sort;
  if (!sort) return entries;
  return [...entries].sort((a, b) => {
    if (sort.foldersFirst && a.kind !== b.kind) return a.kind === "directory" ? -1 : 1;
    const value = (entry: BrowseEntry) => sort.field === "name" ? entry.name : sort.field === "type" ? entry.extension : sort.field === "size" ? entry.size : entry.modifiedAt;
    const av = value(a), bv = value(b);
    const compared = av === bv ? 0 : av === undefined ? -1 : bv === undefined ? 1 : av < bv ? -1 : 1;
    const tie = a.name === b.name ? 0 : a.name < b.name ? -1 : 1;
    return (compared || tie) * (sort.direction === "desc" ? -1 : 1);
  });
}
function publishFixturePage(session: MockBrowseSession, requestId: string, enumerationId: string, pageSize: number, query: BrowseQuerySpecV1, raw: BrowseEntry[]): BrowsePage {
  const all = sortedMockEntries(raw.filter((entry) => (query.entryKind === "all" || entry.kind === query.entryKind)
    && (!query.text || entry.name.toLowerCase().includes(query.text.trim().toLowerCase()))), query);
  if (query.sort && raw.length > 20_000) throw Error("browse_sort_capacity_exceeded");
  const start = session.enumeration?.nextIndex ?? 0;
  const entries = all.slice(start, start + Math.max(1, Math.min(256, pageSize)));
  const complete = start + entries.length >= all.length;
  if (session.enumeration) session.enumeration.nextIndex = start + entries.length;
  for (const entry of entries) {
    managementEntries.set(entry.ref.entryId, entry);
    session.entries.set(entry.ref.entryId, { enumerationId, pathRefId: entry.pathRef?.id });
    if (entry.pathRef) session.pathRefs.add(entry.pathRef.id);
  }
  return { sessionId: session.sessionId, requestId, enumerationId, entries, completion: complete ? "complete" : "partial",
    ...(complete ? { knownCount: all.length } : { nextCursor: session.enumeration?.cursor }) };
}

function makePage(
  session: MockBrowseSession,
  requestId: string,
  enumerationId: string,
  pageSize: number,
  query: BrowseQuerySpecV1
): BrowsePage {
  const fixtureEntries = session.routingHint && managementFixturePage(session.sessionId, session.routingHint, enumerationId);
  if (fixtureEntries) {
    return publishFixturePage(session, requestId, enumerationId, pageSize, query, fixtureEntries);
  }
  if (isW303QueryGap(query)) {
    return makeW303QueryPage(session, requestId, enumerationId);
  }
  if (isW211IntegratedFixtureEnabled()) {
    if (query.sort && W211_BROWSE_TOTAL > 20_000) throw Error("browse_sort_capacity_exceeded");
    return makeW211Page(session, requestId, enumerationId, pageSize, query);
  }
  const allEntries = [
    {
      ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-file` },
      name: "mock-file.txt",
      displayPath: "mock-file.txt",
      kind: "file" as const,
      extension: "txt",
      size: 12,
      modifiedAt: 1,
      createdAt: 1,
      materialization: isThumbnailFixtureEnabled() ? "boundary_readable" as const : "unknown" as const
    },
    {
      ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-folder` },
      pathRef: { id: `${enumerationId}-folder-path` },
      name: "mock-folder",
      displayPath: "mock-folder",
      kind: "directory" as const,
      materialization: "unknown" as const
    },
    {
      ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-notes` },
      name: "notes.md",
      displayPath: "notes.md",
      kind: "file" as const,
      extension: "md",
      size: 24,
      modifiedAt: 2,
      createdAt: 2,
      materialization: "unknown" as const
    },
    {
      ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-images` },
      pathRef: { id: `${enumerationId}-images-path` },
      name: "images-folder",
      displayPath: "images-folder",
      kind: "directory" as const,
      materialization: "unknown" as const
    },
    ...(isW304FixtureEnabled()
      ? [
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-markdown` },
          name: "W3-04-hostile.md",
          displayPath: "W3-04-hostile.md",
          kind: "file" as const,
          extension: "md",
          size: 1_280,
          modifiedAt: 3,
          createdAt: 3,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-source` },
          name: "preview-fixture.rs",
          displayPath: "preview-fixture.rs",
          kind: "file" as const,
          extension: "rs",
          size: 768,
          modifiedAt: 4,
          createdAt: 4,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-partial` },
          name: "bounded-prefix.txt",
          displayPath: "bounded-prefix.txt",
          kind: "file" as const,
          extension: "txt",
          size: 700_000,
          modifiedAt: 5,
          createdAt: 5,
          materialization: "boundary_readable" as const
        }
      ]
      : []),
    ...(isW305FixtureEnabled()
      ? [
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-json` },
          name: "structured-sample.json",
          displayPath: "structured-sample.json",
          kind: "file" as const,
          extension: "json",
          size: 1_024,
          modifiedAt: 6,
          createdAt: 6,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-yaml` },
          name: "structured-config.yaml",
          displayPath: "structured-config.yaml",
          kind: "file" as const,
          extension: "yaml",
          size: 1_024,
          modifiedAt: 7,
          createdAt: 7,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-xml` },
          name: "structured-markup.xml",
          displayPath: "structured-markup.xml",
          kind: "file" as const,
          extension: "xml",
          size: 1_024,
          modifiedAt: 8,
          createdAt: 8,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-csv` },
          name: "structured-records.csv",
          displayPath: "structured-records.csv",
          kind: "file" as const,
          extension: "csv",
          size: 1_024,
          modifiedAt: 9,
          createdAt: 9,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-tsv` },
          name: "structured-records.tsv",
          displayPath: "structured-records.tsv",
          kind: "file" as const,
          extension: "tsv",
          size: 1_024,
          modifiedAt: 10,
          createdAt: 10,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-structured-partial` },
          name: "structured-partial.json",
          displayPath: "structured-partial.json",
          kind: "file" as const,
          extension: "json",
          size: 700_000,
          modifiedAt: 11,
          createdAt: 11,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-table-partial` },
          name: "table-partial.csv",
          displayPath: "table-partial.csv",
          kind: "file" as const,
          extension: "csv",
          size: 700_000,
          modifiedAt: 12,
          createdAt: 12,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-malformed` },
          name: "malformed-structured.json",
          displayPath: "malformed-structured.json",
          kind: "file" as const,
          extension: "json",
          size: 128,
          modifiedAt: 13,
          createdAt: 13,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-fallback` },
          name: "unsupported-structured.bin",
          displayPath: "unsupported-structured.bin",
          kind: "file" as const,
          extension: "bin",
          size: 8_192,
          modifiedAt: 14,
          createdAt: 14,
          materialization: "boundary_readable" as const
        }
      ]
      : []),
    ...(isW306FixtureEnabled()
      ? [
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-png` },
          name: "image-sample.png",
          displayPath: "image-sample.png",
          kind: "file" as const,
          extension: "png",
          size: 1_024,
          modifiedAt: 16,
          createdAt: 16,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-jpeg` },
          name: "image-sample.jpg",
          displayPath: "image-sample.jpg",
          kind: "file" as const,
          extension: "jpg",
          size: 2_048,
          modifiedAt: 17,
          createdAt: 17,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-partial` },
          name: "image-bounded.png",
          displayPath: "image-bounded.png",
          kind: "file" as const,
          extension: "png",
          size: 12_582_912,
          modifiedAt: 18,
          createdAt: 18,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-corrupt` },
          name: "image-corrupt.png",
          displayPath: "image-corrupt.png",
          kind: "file" as const,
          extension: "png",
          size: 96,
          modifiedAt: 19,
          createdAt: 19,
          materialization: "boundary_readable" as const
        }
      ]
      : []),
    ...(isW308FixtureEnabled()
      ? [
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-archive-sample` },
          name: "archive-sample.zip",
          displayPath: "archive-sample.zip",
          kind: "file" as const,
          extension: "zip",
          size: 4_096,
          modifiedAt: 22,
          createdAt: 22,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-archive-partial` },
          name: "archive-partial.zip",
          displayPath: "archive-partial.zip",
          kind: "file" as const,
          extension: "zip",
          size: 12_582_912,
          modifiedAt: 23,
          createdAt: 23,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-archive-hostile` },
          name: "archive-hostile.zip",
          displayPath: "archive-hostile.zip",
          kind: "file" as const,
          extension: "zip",
          size: 8_192,
          modifiedAt: 24,
          createdAt: 24,
          materialization: "boundary_readable" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-archive-corrupt` },
          name: "archive-corrupt.zip",
          displayPath: "archive-corrupt.zip",
          kind: "file" as const,
          extension: "zip",
          size: 128,
          modifiedAt: 25,
          createdAt: 25,
          materialization: "boundary_readable" as const
        }
      ]
      : []),
    ...(isW307FixtureEnabled()
      ? [
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-w3-07-empty-folder` },
          pathRef: { id: `${enumerationId}-w3-07-empty-folder-path` },
          name: "w3-07-empty-folder",
          displayPath: "w3-07-empty-folder",
          kind: "directory" as const,
          materialization: "unknown" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-w3-07-mixed-folder` },
          pathRef: { id: `${enumerationId}-w3-07-mixed-folder-path` },
          name: "w3-07-mixed-folder",
          displayPath: "w3-07-mixed-folder",
          kind: "directory" as const,
          materialization: "unknown" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-w3-07-1000-folder` },
          pathRef: { id: `${enumerationId}-w3-07-1000-folder-path` },
          name: "w3-07-1000-folder",
          displayPath: "w3-07-1000-folder",
          kind: "directory" as const,
          materialization: "unknown" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-w3-07-10000-folder` },
          pathRef: { id: `${enumerationId}-w3-07-10000-folder-path` },
          name: "w3-07-10000-folder",
          displayPath: "w3-07-10000-folder",
          kind: "directory" as const,
          materialization: "unknown" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-w3-07-100000-folder` },
          pathRef: { id: `${enumerationId}-w3-07-100000-folder-path` },
          name: "w3-07-100000-folder",
          displayPath: "w3-07-100000-folder",
          kind: "directory" as const,
          materialization: "unknown" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-w3-07-100001-folder` },
          pathRef: { id: `${enumerationId}-w3-07-100001-folder-path` },
          name: "w3-07-100001-folder",
          displayPath: "w3-07-100001-folder",
          kind: "directory" as const,
          materialization: "unknown" as const
        },
        {
          ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-w3-07-deadline-folder` },
          pathRef: { id: `${enumerationId}-w3-07-deadline-folder-path` },
          name: "w3-07-deadline-folder",
          displayPath: "w3-07-deadline-folder",
          kind: "directory" as const,
          materialization: "unknown" as const
        }
      ]
      : [])
  ].filter((entry) => {
    const kindMatches = query.entryKind === "all" || entry.kind === query.entryKind;
    const text = query.text?.trim().toLocaleLowerCase() ?? "";
    return kindMatches && (text.length === 0 || entry.name.toLocaleLowerCase().includes(text));
  });
  const requestedLimit = Math.max(1, Math.min(256, Number.isFinite(pageSize) ? pageSize : 1));
  // Keep the legacy W2 browser scenes progressive after W2-08 expanded this
  // fixture to four entries; production requests still use their requested limit.
  const limit = isW204SourceOwnerFixtureEnabled() ? Math.min(2, requestedLimit) : requestedLimit;
  const offset = session.enumeration?.enumerationId === enumerationId
    ? session.enumeration.nextIndex
    : 0;
  const entries = sortedMockEntries(allEntries, query).slice(offset, offset + limit);
  const complete = offset + entries.length >= allEntries.length;
  if (session.enumeration?.enumerationId === enumerationId) {
    session.enumeration = {
      ...session.enumeration,
      nextIndex: offset + entries.length,
      ...(complete ? { cursor: undefined } : {})
    };
  }
  for (const entry of entries) {
    managementEntries.set(entry.ref.entryId, entry);
    if (entry.pathRef !== undefined) session.pathRefs.add(entry.pathRef.id);
    session.entries.set(entry.ref.entryId, {
      enumerationId,
      ...(entry.pathRef === undefined ? {} : { pathRefId: entry.pathRef.id })
    });
  }
  return {
    sessionId: session.sessionId,
    requestId,
    enumerationId,
    entries,
    ...(complete ? {} : { nextCursor: session.enumeration?.cursor }),
    completion: complete ? "complete" : "partial",
    ...(complete ? { knownCount: allEntries.length } : {})
  };
}

function isW303QueryGap(query: BrowseQuerySpecV1) {
  return isW303FixtureEnabled() && query.text?.trim().toLowerCase() === "w3-03-gap";
}

function makeW303QueryPage(
  session: MockBrowseSession,
  requestId: string,
  enumerationId: string
): BrowsePage {
  const enumeration = session.enumeration;
  const pageIndex = enumeration?.nextIndex ?? 0;
  const entry = (id: string): BrowseEntry => ({
    ref: { kind: "ephemeral", browseSessionId: session.sessionId, entryId: `${enumerationId}-${id}` },
    name: `w3-03-gap-${id}.txt`,
    displayPath: `w3-03-gap-${id}.txt`,
    kind: "file",
    extension: "txt",
    size: 12,
    modifiedAt: 1,
    createdAt: 1,
    materialization: "unknown"
  });
  const entries = pageIndex === 0 ? [entry("a")] : pageIndex === 1 ? [] : [entry("b")];
  const complete = pageIndex >= 2;
  const nextIndex = pageIndex + 1;
  if (enumeration?.enumerationId === enumerationId) {
    session.enumeration = {
      ...enumeration,
      nextIndex,
      ...(complete ? { cursor: undefined } : {})
    };
  }
  for (const item of entries) session.entries.set(item.ref.entryId, { enumerationId });
  return {
    sessionId: session.sessionId,
    requestId,
    enumerationId,
    entries,
    ...(complete ? {} : { nextCursor: session.enumeration?.cursor }),
    completion: complete ? "complete" : "partial",
    ...(complete ? { knownCount: 2 } : {})
  };
}

function makeW211Page(
  session: MockBrowseSession,
  requestId: string,
  enumerationId: string,
  pageSize: number,
  query: BrowseQuerySpecV1
): BrowsePage {
  const enumeration = session.enumeration;
  const requestedLimit = Math.max(1, Math.min(256, Number.isFinite(pageSize) ? pageSize : 1));
  const pathRefId = enumeration?.pathRefId ?? session.rootPathRef.id;
  if (pathRefId !== session.rootPathRef.id) {
    const childEntries = Array.from({ length: Math.min(requestedLimit, 8) }, (_, index) => ({
      ref: { kind: "ephemeral" as const, browseSessionId: session.sessionId, entryId: `${enumerationId}-child-${index}` },
      name: `w2-11-child-item-${String(index + 1).padStart(2, "0")}.txt`,
      displayPath: `w2-11-child-item-${String(index + 1).padStart(2, "0")}.txt`,
      kind: "file" as const,
      extension: "txt",
      size: 2_048 + index,
      modifiedAt: index + 1,
      createdAt: index + 1,
      materialization: "boundary_readable" as const
    }));
    for (const entry of childEntries) {
      session.entries.set(entry.ref.entryId, { enumerationId });
    }
    if (enumeration?.enumerationId === enumerationId) {
      session.enumeration = { ...enumeration, nextIndex: childEntries.length, cursor: undefined };
    }
    return {
      sessionId: session.sessionId,
      requestId,
      enumerationId,
      entries: childEntries,
      completion: "complete",
      knownCount: childEntries.length
    };
  }

  const rawStart = enumeration?.enumerationId === enumerationId ? enumeration.nextIndex : 0;
  const text = query.text?.trim().toLowerCase() ?? "";
  const progressiveQuery = text.length > 0;
  const rawEnd = Math.min(
    W211_BROWSE_TOTAL,
    rawStart + (progressiveQuery ? W211_SCAN_BUDGET : requestedLimit)
  );
  const entries: BrowsePage["entries"] = [];
  for (let rawIndex = rawStart; rawIndex < rawEnd && entries.length < requestedLimit; rawIndex += 1) {
    const entry = w211BrowseEntry(session.sessionId, enumerationId, rawIndex, query);
    if (entry === undefined) continue;
    entries.push(entry);
  }
  const complete = rawEnd >= W211_BROWSE_TOTAL;
  const nextIndex = rawEnd;
  if (enumeration?.enumerationId === enumerationId) {
    session.enumeration = {
      ...enumeration,
      nextIndex,
      ...(complete ? { cursor: undefined } : {})
    };
  }
  for (const entry of entries) {
    managementEntries.set(entry.ref.entryId, entry);
    if (entry.pathRef !== undefined) session.pathRefs.add(entry.pathRef.id);
    session.entries.set(entry.ref.entryId, {
      enumerationId,
      ...(entry.pathRef === undefined ? {} : { pathRefId: entry.pathRef.id })
    });
  }
  const stats = w211Stats();
  if (stats) {
    stats.browsePageCalls += 1;
    stats.browsePageLengths.push(entries.length);
    stats.browseScanEnds.push(rawEnd);
    stats.browseQueries.push(text);
    if (stats.browseFirstPageSnapshots.length < 400) {
      stats.browseFirstPageSnapshots.push({
        query: text,
        entries: entries.length,
        completion: complete ? "complete" : "partial",
        hasCursor: !complete,
        ...(complete ? { knownCount: w211BrowseMatchCount(query) } : {})
      });
    }
  }
  return {
    sessionId: session.sessionId,
    requestId,
    enumerationId,
    entries,
    ...(complete ? {} : { nextCursor: session.enumeration?.cursor }),
    completion: complete ? "complete" : "partial",
    ...(complete ? { knownCount: w211BrowseMatchCount(query) } : {})
  };
}

function w211BrowseEntry(
  sessionId: string,
  enumerationId: string,
  rawIndex: number,
  query: BrowseQuerySpecV1
): BrowsePage["entries"][number] | undefined {
  const text = query.text?.trim().toLowerCase() ?? "";
  const isDirectory = rawIndex === 1 && text.length === 0;
  const matchesText = text.length === 0
    || (text === "late-sentinel" && rawIndex === W211_LATE_SENTINEL_INDEX)
    || (text === "slow-a" && rawIndex === 0)
    || (text === "slow-b" && rawIndex === 0);
  const matchesKind = query.entryKind === "all"
    || (query.entryKind === "directory" && isDirectory)
    || (query.entryKind === "file" && !isDirectory);
  if (!matchesText || !matchesKind) return undefined;
  if (isDirectory) {
    return {
      ref: { kind: "ephemeral", browseSessionId: sessionId, entryId: `${enumerationId}-folder-${rawIndex}` },
      pathRef: { id: `${enumerationId}-child-path` },
      name: "w2-11-child-folder",
      displayPath: "w2-11-child-folder",
      kind: "directory",
      materialization: "unknown"
    };
  }
  const prefix = text === "late-sentinel"
    ? "late-sentinel"
    : text === "slow-a"
      ? "slow-a"
      : text === "slow-b"
        ? "slow-b"
        : "w2-11-browse-item";
  return {
    ref: { kind: "ephemeral", browseSessionId: sessionId, entryId: `${enumerationId}-entry-${rawIndex}` },
    name: `${prefix}-${String(rawIndex + 1).padStart(6, "0")}.txt`,
    displayPath: `${prefix}-${String(rawIndex + 1).padStart(6, "0")}.txt`,
    kind: "file",
    extension: "txt",
    size: 4_096 + rawIndex,
    modifiedAt: rawIndex + 1,
    createdAt: rawIndex + 1,
    materialization: "boundary_readable"
  };
}

function w211BrowseMatchCount(query: BrowseQuerySpecV1) {
  const text = query.text?.trim().toLowerCase() ?? "";
  if (text === "late-sentinel" || text === "slow-a" || text === "slow-b") return 1;
  if (text === "impossible-match") return 0;
  if (query.entryKind === "directory") return 1;
  if (query.entryKind === "file") return W211_BROWSE_TOTAL - 1;
  return W211_BROWSE_TOTAL;
}

function cancelEnumeration(request: MockArgs) {
  const session = getSession(String(request?.sessionId ?? ""));
  const hasEnumeration = request?.enumeration !== undefined;
  const hasRequestId = request?.requestId !== undefined;
  if (hasEnumeration === hasRequestId
    || (hasRequestId && String(request?.requestId ?? "").length === 0)) {
    throw new Error("browse_cancel_requires_exactly_one_identity");
  }
  const requestedEnumerationId = (request?.enumeration as { enumerationId?: string } | undefined)?.enumerationId;
  const requestedRequestId = request?.requestId;
  if (!session.enumeration
    || (requestedEnumerationId !== undefined
      && session.enumeration.enumerationId !== String(requestedEnumerationId))
    || (requestedEnumerationId === undefined
      && session.enumeration.requestId !== String(requestedRequestId ?? ""))) {
    throw new Error("browse_enumeration_stale");
  }
  const enumerationId = session.enumeration.enumerationId;
  session.enumeration = undefined;
  invalidateEntriesForEnumeration(session, enumerationId);
}

function releasePage(request: MockArgs) {
  const page = request?.page as BrowsePage | undefined;
  if (!page || typeof page.sessionId !== "string") throw new Error("browse_page_invalid");
  const session = getSession(page.sessionId);
  for (const entry of page.entries) {
    if (entry.ref.browseSessionId !== page.sessionId) continue;
    const stored = session.entries.get(entry.ref.entryId);
    if (stored === undefined || stored.enumerationId !== page.enumerationId) continue;
    session.entries.delete(entry.ref.entryId);
    if (stored.pathRefId !== undefined) removePathIfUnpinned(session, stored.pathRefId);
  }
}

function releasePath(request: MockArgs) {
  const session = getSession(String(request?.sessionId ?? ""));
  const pathRef = request?.pathRef as { id?: string } | undefined;
  if (typeof pathRef?.id !== "string" || !session.pathRefs.has(pathRef.id)) {
    throw new Error("browse_path_ref_invalid");
  }
  if (pathRef.id === session.rootPathRef.id) return;
  session.retainedPathRefs.delete(pathRef.id);
  removePathIfUnpinned(session, pathRef.id);
}

function invalidateEntriesForEnumeration(session: MockBrowseSession, enumerationId: string) {
  for (const [entryId, entry] of session.entries) {
    if (entry.enumerationId !== enumerationId) continue;
    session.entries.delete(entryId);
    if (entry.pathRefId !== undefined) removePathIfUnpinned(session, entry.pathRefId);
  }
}

function removePathIfUnpinned(session: MockBrowseSession, pathRefId: string) {
  if (session.retainedPathRefs.has(pathRefId)) return;
  for (const entry of session.entries.values()) {
    if (entry.pathRefId === pathRefId) return;
  }
  session.pathRefs.delete(pathRefId);
}

function retainPath(request: MockArgs) {
  const session = getSession(String(request?.sessionId ?? ""));
  const pathRef = request?.pathRef as { id?: string } | undefined;
  if (typeof pathRef?.id !== "string" || !session.pathRefs.has(pathRef.id)) {
    throw new Error("browse_path_ref_invalid");
  }
  session.retainedPathRefs.add(pathRef.id);
}

function disposeBrowse(request: MockArgs) {
  const sessionId = String(request?.sessionId ?? "");
  if (!sessions.delete(sessionId)) throw new Error("browse_session_not_found");
  const stats = w211Stats();
  if (stats) stats.browseSessionsDisposed += 1;
  for (const [monitorId, ownerSessionId] of monitors) {
    if (ownerSessionId === sessionId) monitors.delete(monitorId);
  }
}

function startChange(request: ChangeStartRequest): ChangeStartResponse {
  const session = getSession(request.sessionId);
  if (!session.pathRefs.has(request.pathRef.id)) throw new Error("browse_path_ref_invalid");
  const monitorId = id("change");
  monitors.set(monitorId, request.sessionId);
  return { monitorId, sessionId: request.sessionId, pathRef: request.pathRef };
}

function pendingChange(request: MockArgs): ChangePendingResponse | null {
  if (!monitors.has(String(request?.monitorId ?? ""))) throw new Error("ephemeral_change_monitor_not_found");
  return null;
}

function readEligibility(request: ReadEligibilityRequest): ReadEligibilityResponse {
  if ("path" in (request.source as Record<string, unknown>)) throw new Error("content_source_not_supported");
  return {
    source: request.source,
    eligibility: request.source.kind === "host_provided" ? "source_not_supported" : "eligible"
  };
}

function isW206ThumbnailFixtureEnabled() {
  return typeof window !== "undefined"
    && (new URLSearchParams(window.location.search).get("w2-06-browser-fixture") === "grid"
      || isW211IntegratedFixtureEnabled());
}

function isThumbnailFixtureEnabled() {
  return isW206ThumbnailFixtureEnabled();
}

async function thumbnailRequest(request: ThumbnailRequest) {
  if (!isThumbnailRequestShape(request)) {
    throw new Error("thumbnail_request_invalid");
  }
  if (request.source.kind === "ephemeral") {
    if (request.sessionId !== undefined && request.sessionId !== request.source.browseSessionId) {
      throw new Error("thumbnail_request_invalid");
    }
    const session = sessions.get(request.source.browseSessionId);
    if (session === undefined || session.disposed
      || !session.entries.has(request.source.entryId)) {
      throw new Error("thumbnail_source_unavailable");
    }
  }
  if (!isThumbnailFixtureEnabled()) {
    throw new Error("thumbnail_renderer_unsupported_browser_mock");
  }
  const stats = w211Stats();
  if (stats) {
    stats.thumbnailRequests += 1;
    stats.activeThumbnailRequests += 1;
    stats.thumbnailVariants.push(request.variant);
  }
  try {
    await new Promise((resolve) => setTimeout(resolve, 50));
    return {
      cacheKey: `browser-mock-thumbnail:${request.source.kind}:${request.variant}`,
      bytes: new Uint8Array(MOCK_THUMBNAIL_BYTES)
    };
  } finally {
    if (stats) stats.activeThumbnailRequests = Math.max(0, stats.activeThumbnailRequests - 1);
  }
}

function recordW206ThumbnailCancel() {
  if (!isW206ThumbnailFixtureEnabled() || typeof window === "undefined") return;
  const testWindow = window as Window & { __zcW206ThumbnailCancels?: number };
  testWindow.__zcW206ThumbnailCancels = (testWindow.__zcW206ThumbnailCancels ?? 0) + 1;
  const stats = w211Stats();
  if (stats) stats.thumbnailCancels += 1;
}

function isThumbnailRequestShape(request: ThumbnailRequest): request is ThumbnailRequest {
  if (!request || typeof request !== "object") return false;
  const keys = Object.keys(request as unknown as Record<string, unknown>);
  const allowedKeys = new Set(["requestId", "source", "variant", "workClass", "sessionId"]);
  if (keys.some((key) => !allowedKeys.has(key))) return false;
  if (!isOpaqueId(request.requestId)
    || !["small", "medium", "large"].includes(request.variant)
    || !["foreground", "interactive", "background"].includes(request.workClass)) {
    return false;
  }
  if (request.sessionId !== undefined && !isOpaqueId(request.sessionId)) return false;
  if (!request.source || typeof request.source !== "object") return false;
  const source = request.source as Record<string, unknown>;
  if (source.kind === "managed") {
    return Object.keys(source).every((key) => key === "kind" || key === "fileId")
      && isOpaqueId(source.fileId)
      && !looksLikePath(source.fileId);
  }
  if (source.kind === "ephemeral") {
    return Object.keys(source).every((key) =>
      key === "kind" || key === "browseSessionId" || key === "entryId")
      && isOpaqueId(source.browseSessionId)
      && isOpaqueId(source.entryId)
      && !looksLikePath(source.browseSessionId)
      && !looksLikePath(source.entryId);
  }
  return false;
}

function encodeThumbnailIpcResponse(cacheKey: string, artifactBytes: Uint8Array): ArrayBuffer {
  const cacheKeyBytes = new TextEncoder().encode(cacheKey);
  const payload = new Uint8Array(13 + cacheKeyBytes.byteLength + artifactBytes.byteLength);
  payload.set([0x5a, 0x43, 0x54, 0x48, 1], 0);
  const view = new DataView(payload.buffer);
  view.setUint32(5, cacheKeyBytes.byteLength, true);
  view.setUint32(9, artifactBytes.byteLength, true);
  payload.set(cacheKeyBytes, 13);
  payload.set(artifactBytes, 13 + cacheKeyBytes.byteLength);
  return payload.buffer;
}

function encodePreviewAssetIpcResponse(artifact: PreviewAssetArtifact): ArrayBuffer {
  const mediaTypeBytes = new TextEncoder().encode(artifact.mediaType);
  const payload = new Uint8Array(13 + mediaTypeBytes.byteLength + artifact.bytes.byteLength);
  payload.set([0x5a, 0x43, 0x41, 0x53, 1], 0);
  const view = new DataView(payload.buffer);
  view.setUint32(5, mediaTypeBytes.byteLength, true);
  view.setUint32(9, artifact.bytes.byteLength, true);
  payload.set(mediaTypeBytes, 13);
  payload.set(artifact.bytes, 13 + mediaTypeBytes.byteLength);
  return payload.buffer;
}

function isOpaqueId(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= 256 && !value.includes("\0");
}

function looksLikePath(value: string) {
  return value.includes("/") || value.includes("\\") || value.startsWith("C:");
}

function createPreview(request: PreviewCreateRequest): PreviewSnapshot {
  if (request.hostKind !== "zen_floating" && request.hostKind !== "zen_pinned") {
    throw new Error("preview_host_not_activated");
  }
  const previewId = id("preview");
  const snapshot: PreviewSnapshot = {
    previewId,
    sessionId: previewId,
    requestId: request.requestId,
    source: request.source,
    hostKind: request.hostKind,
    state: "idle",
    effectiveCapabilities: metadataCapabilities()
  };
  previews.set(previewId, { snapshot });
  const fixture = w302Stats();
  if (fixture !== null) {
    fixture.createHostKinds.push(request.hostKind);
    fixture.activeBackendHostKinds[previewId] = request.hostKind;
    fixture.activeBackendSources[previewId] = snapshot.source;
  }
  return parsePreviewSnapshot(snapshot);
}

function previewSnapshot(request: MockArgs): PreviewSnapshot {
  const record = getPreview(String(request?.previewId ?? ""));
  const snapshot = record.snapshot;
  const next = record.pendingSnapshots?.shift();
  if (next !== undefined) record.snapshot = next;
  recordW307Snapshot(snapshot);
  return parsePreviewSnapshot(snapshot);
}

function previewStart(request: MockArgs): PreviewSnapshot | Promise<PreviewSnapshot> {
  const record = getPreview(String(request?.previewId ?? ""));
  const snapshot = record.snapshot;
  const terminalCode = w309TerminalCode(snapshot.source);
  if (terminalCode !== null) {
    const stats = w309Stats();
    if (stats !== null) {
      stats.terminalStarts += 1;
      stats.terminalCodes.push(terminalCode);
    }
    throw new Error(terminalCode);
  }
  const metadata = mockMetadata(snapshot.source);
  const sourceKey = previewSourceKey(snapshot.source);
  const rich = (isW307FixtureEnabled() ? w307Representation(snapshot.source) : null)
    ?? (isW308FixtureEnabled() ? w308Representation(snapshot.source) : null)
    ?? (isW306FixtureEnabled() ? w306Representation(snapshot.source) : null)
    ?? (isW305FixtureEnabled() ? w305Representation(snapshot.source) : null)
    ?? (isW304FixtureEnabled() ? w304Representation(snapshot.source) : null);
  const sourceVersion = rich?.providerId === "builtin.folder"
    ? `browser-w307-${sourceKey}`
    : rich?.providerId === "builtin.archive-zip"
    ? `browser-w308-${sourceKey}`
    : rich?.providerId === "builtin.image"
    ? `browser-w306-${sourceKey}`
    : rich?.providerId?.startsWith("builtin.structured") || rich?.providerId?.startsWith("builtin.table")
    ? `browser-w305-${sourceKey}`
    : rich !== null
    ? `browser-w304-${sourceKey}`
    : `browser-mock-v1-${snapshot.source.kind}`;
  const capabilities = rich === null
    ? metadataCapabilities()
    : rich.providerId === "builtin.folder"
    ? metadataCapabilities()
    : { ...metadataCapabilities(), canSelectText: true };
  const fixture304 = w304Stats();
  if (fixture304 !== null) {
    fixture304.starts += 1;
    if (rich === null) fixture304.fallbackStarts += 1;
    else fixture304.richStarts += 1;
  }
  const fixture307 = w307Stats();
  if (fixture307 !== null) {
    fixture307.starts += 1;
    fixture307.lastSourceKey = sourceKey;
    if (rich?.providerId === "builtin.folder" && rich.representation.family === "folder_summary") {
      fixture307.richStarts += 1;
      fixture307.lastSummary = rich.representation.encodedSummary;
    } else {
      fixture307.fallbackStarts += 1;
      fixture307.lastSummary = null;
    }
  }
  const readySnapshot: PreviewSnapshot = {
    ...snapshot,
    state: "ready",
    sourceVersion,
    representation: {
      sourceVersion,
      representation: rich?.representation ?? { family: "metadata", metadata },
      completeness: rich?.completeness ?? "complete",
      warnings: rich === null ? [{ kind: "metadata_fallback" }] : [],
      capabilities
    },
    effectiveCapabilities: capabilities,
    ...(rich === null ? {} : { activeProviderId: rich.providerId })
  };
  const progressive = rich?.providerId === "builtin.folder" && rich.representation.family === "folder_summary"
    ? w307ProgressiveSnapshots(readySnapshot)
    : null;
  if (progressive !== null) {
    record.snapshot = progressive[0]!;
    record.pendingSnapshots = progressive.slice(1);
  }
  const fixture = w302Stats();
  if (fixture === null) {
    record.snapshot = readySnapshot;
    record.pendingSnapshots = undefined;
    return parsePreviewSnapshot(record.snapshot);
  }

  fixture.started += 1;
  return new Promise<PreviewSnapshot>((resolve) => {
    pendingPreviewStarts.push({
      previewId: snapshot.previewId,
      requestId: snapshot.requestId,
      source: snapshot.source,
      snapshot: readySnapshot,
      resolve
    });
    fixture.pendingStartCount = pendingPreviewStarts.length;
  });
}

function cancelPreview(request: MockArgs) {
  const fixture = w302Stats();
  if (fixture !== null) fixture.cancelCalls += 1;
  const record = getPreview(String(request?.previewId ?? ""));
  record.snapshot = { ...record.snapshot, state: "cancelled" };
  return true;
}

function disposePreview(request: MockArgs) {
  const previewId = String(request?.previewId ?? "");
  const fixture = w302Stats();
  if (fixture !== null) fixture.disposeCalls += 1;
  if (fixture !== null) delete fixture.activeBackendHostKinds[previewId];
  if (fixture !== null) delete fixture.activeBackendSources[previewId];
  if (!previews.delete(previewId) && fixture === null) throw new Error("preview_session_not_found");
}

function switchPreviewSource(request: MockArgs): PreviewSnapshot {
  const fixture = w302Stats();
  if (fixture !== null) fixture.switchCalls += 1;
  const record = getPreview(String(request?.previewId ?? ""));
  const source = request?.source as PreviewSnapshot["source"];
  record.snapshot = {
    ...record.snapshot,
    requestId: String(request?.requestId ?? ""),
    source,
    state: "resolving",
    sourceVersion: undefined,
    representation: undefined
  };
  if (fixture !== null) fixture.activeBackendSources[String(request?.previewId ?? "")] = source;
  record.pendingSnapshots = undefined;
  return parsePreviewSnapshot(record.snapshot);
}

function resolveNextPreviewStart() {
  const pending = pendingPreviewStarts.shift();
  const fixture = w302Stats();
  if (pending === undefined) {
    if (fixture !== null) fixture.pendingStartCount = 0;
    return;
  }
  if (fixture !== null) {
    fixture.pendingStartCount = pendingPreviewStarts.length;
    fixture.resolved += 1;
  }
  const record = previews.get(pending.previewId);
  if (record !== undefined
    && record.snapshot.requestId === pending.requestId
    && samePreviewSource(record.snapshot.source, pending.source)
    && record.snapshot.state !== "cancelled") {
    record.snapshot = pending.snapshot;
    record.pendingSnapshots = undefined;
  } else if (fixture !== null) {
    fixture.lateStarts += 1;
  }
  pending.resolve(parsePreviewSnapshot(pending.snapshot));
}

function resolveAllPreviewStarts() {
  while (pendingPreviewStarts.length > 0) resolveNextPreviewStart();
}

async function previewAssetRequest(request: PreviewAssetRequest): Promise<PreviewAssetArtifact> {
  const record = previews.get(request.previewId);
  if (record === undefined) throw new Error("preview_asset_preview_not_found");
  const sourceKey = previewSourceKey(record.snapshot.source);
  const descriptor = w306ImageDescriptor(record.snapshot.source);
  if (descriptor === null
    || request.requestId !== record.snapshot.requestId
    || request.sourceVersion !== `browser-w306-${sourceKey}`
    || request.assetToken !== `w306-asset-${sourceKey}`) {
    throw new Error("preview_asset_invalid_or_stale");
  }
  const artifact: PreviewAssetArtifact = {
    mediaType: descriptor.mediaType,
    bytes: descriptor.bytes.slice()
  };
  const stats = w306Stats();
  if (stats === null) return artifact;
  stats.assetRequests.push({ ...request });
  return new Promise((resolve) => {
    pendingPreviewAssets.push({ artifact, resolve });
    stats.pendingAssetCount = pendingPreviewAssets.length;
  });
}

function resolveNextPreviewAsset() {
  const pending = pendingPreviewAssets.shift();
  const stats = w306Stats();
  if (pending === undefined) {
    if (stats !== null) stats.pendingAssetCount = 0;
    return;
  }
  if (stats !== null) {
    stats.pendingAssetCount = pendingPreviewAssets.length;
    stats.resolvedAssets += 1;
  }
  pending.resolve({ ...pending.artifact, bytes: pending.artifact.bytes.slice() });
}

function resolveAllPreviewAssets() {
  while (pendingPreviewAssets.length > 0) resolveNextPreviewAsset();
}

function samePreviewSource(left: PreviewSnapshot["source"], right: PreviewSnapshot["source"]) {
  if (left.kind !== right.kind) return false;
  if (left.kind === "managed" && right.kind === "managed") return left.fileId === right.fileId;
  if (left.kind === "ephemeral" && right.kind === "ephemeral") {
    return left.browseSessionId === right.browseSessionId && left.entryId === right.entryId;
  }
  return left.kind === "host_provided" && right.kind === "host_provided" && left.hostToken === right.hostToken;
}

function mockMetadata(source: PreviewSnapshot["source"]): PreviewMetadata {
  const fixtureMetadata309 = w309Metadata(source);
  if (fixtureMetadata309 !== null) return fixtureMetadata309;
  const fixtureMetadata307 = w307Metadata(source);
  if (fixtureMetadata307 !== null) return fixtureMetadata307;
  const fixtureMetadata308 = w308Metadata(source);
  if (fixtureMetadata308 !== null) return fixtureMetadata308;
  const fixtureMetadata306 = w306Metadata(source);
  if (fixtureMetadata306 !== null) return fixtureMetadata306;
  const fixtureMetadata305 = w305Metadata(source);
  if (fixtureMetadata305 !== null) return fixtureMetadata305;
  const fixtureMetadata = w304Metadata(source);
  if (fixtureMetadata !== null) return fixtureMetadata;
  return {
    displayName: source.kind === "managed" ? "Managed mock entry" : "Browse mock entry",
    mediaType: null,
    extension: null,
    sizeBytes: 0,
    modifiedAtEpochMs: null,
    materialization: "metadata_only" as const,
    readEligibility: source.kind === "host_provided" ? "source_not_supported" as const : "eligible" as const
  };
}

function w309TerminalCode(source: PreviewSnapshot["source"]) {
  if (!isW309IntegrationFixtureEnabled()) return null;
  const key = previewSourceKey(source);
  if (key.includes("materialization")) return "preview_materialization_required";
  if (key.includes("permission")) return "preview_permission_denied";
  if (key.includes("unavailable")) return "preview_source_unavailable";
  if (key.includes("identity")) return "preview_source_identity_changed";
  return null;
}

function w309Metadata(source: PreviewSnapshot["source"]): PreviewMetadata | null {
  if (!isW309IntegrationFixtureEnabled()) return null;
  const key = previewSourceKey(source);
  const terminal: Array<[string, PreviewMetadata]> = [
    ["materialization", {
      displayName: "W3-09 materialization-required fixture",
      mediaType: "text/plain",
      extension: "txt",
      sizeBytes: 4_096,
      modifiedAtEpochMs: 30,
      materialization: "remote_placeholder",
      readEligibility: "materialization_required"
    }],
    ["permission", {
      displayName: "W3-09 permission-denied fixture",
      mediaType: "text/plain",
      extension: "txt",
      sizeBytes: 4_096,
      modifiedAtEpochMs: 31,
      materialization: "unavailable",
      readEligibility: "permission_required"
    }],
    ["unavailable", {
      displayName: "W3-09 source-unavailable fixture",
      mediaType: "text/plain",
      extension: "txt",
      sizeBytes: 4_096,
      modifiedAtEpochMs: 32,
      materialization: "unavailable",
      readEligibility: "source_unavailable"
    }],
    ["identity", {
      displayName: "W3-09 identity-changed fixture",
      mediaType: "text/plain",
      extension: "txt",
      sizeBytes: 4_096,
      modifiedAtEpochMs: 33,
      materialization: "unavailable",
      readEligibility: "identity_changed"
    }]
  ];
  return terminal.find(([suffix]) => key.includes(suffix))?.[1] ?? null;
}

function w308Metadata(source: PreviewSnapshot["source"]): PreviewMetadata | null {
  if (!isW308FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  if (!isW308SourceKey(key)) return null;
  const definitions: Array<[string, PreviewMetadata]> = [
    ["sample", {
      displayName: "W3-08 complete ZIP archive",
      mediaType: "application/zip",
      extension: "zip",
      sizeBytes: 4_096,
      modifiedAtEpochMs: 22,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["empty", {
      displayName: "W3-08 empty ZIP archive",
      mediaType: "application/zip",
      extension: "zip",
      sizeBytes: 22,
      modifiedAtEpochMs: 22,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["partial", {
      displayName: "W3-08 partial ZIP archive",
      mediaType: "application/zip",
      extension: "zip",
      sizeBytes: 12_582_912,
      modifiedAtEpochMs: 23,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["hostile", {
      displayName: "W3-08 hostile-name ZIP archive",
      mediaType: "application/zip",
      extension: "zip",
      sizeBytes: 8_192,
      modifiedAtEpochMs: 24,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["corrupt", {
      displayName: "W3-08 corrupt ZIP archive",
      mediaType: "application/zip",
      extension: "zip",
      sizeBytes: 128,
      modifiedAtEpochMs: 25,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }]
  ];
  return definitions.find(([suffix]) => key.includes(suffix))?.[1] ?? null;
}

function w308Representation(source: PreviewSnapshot["source"]):
  (Pick<PreviewRepresentationEnvelope, "representation" | "completeness"> & { providerId: string }) | null {
  if (!isW308FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  if (!isW308SourceKey(key)) return null;
  if (key.includes("corrupt")) return null;
  if (key.includes("empty")) {
    return {
      providerId: "builtin.archive-zip",
      representation: {
        family: "archive_tree",
        encodedTree: JSON.stringify({
          version: 1,
          format: "zip",
          progress: { inspectedEntries: 0, state: "complete", limitReason: null },
          totals: {
            entriesObserved: 0,
            filesObserved: 0,
            directoriesObserved: 0,
            compressedBytesObserved: 0,
            uncompressedBytesDeclaredObserved: 0
          },
          root: { kind: "directory", name: "", children: [] },
          warnings: []
        })
      },
      completeness: "complete"
    };
  }
  const complete = {
    version: 1,
    format: "zip",
    progress: { inspectedEntries: 4, state: "complete", limitReason: null },
    totals: {
      entriesObserved: 4,
      filesObserved: 3,
      directoriesObserved: 1,
      compressedBytesObserved: 384,
      uncompressedBytesDeclaredObserved: 768
    },
    root: {
      kind: "directory",
      name: "",
      children: [
        {
          kind: "directory",
          name: "docs",
          children: [
            { kind: "file", name: "readme.md", compressedSize: 120, uncompressedSizeDeclared: 240, compressionMethod: "Deflated" },
            { kind: "file", name: "nested/data.json", compressedSize: 80, uncompressedSizeDeclared: 160, compressionMethod: "Stored" }
          ]
        },
        { kind: "file", name: "photo.jpg", compressedSize: 184, uncompressedSizeDeclared: 368, compressionMethod: "Stored", encrypted: true }
      ]
    },
    warnings: []
  };
  if (key.includes("partial")) {
    return {
      providerId: "builtin.archive-zip",
      representation: {
        family: "archive_tree",
        encodedTree: JSON.stringify({
          ...complete,
          progress: { inspectedEntries: 3, state: "partial", limitReason: "entry_limit" },
          totals: { ...complete.totals, entriesObserved: 3, filesObserved: 3, directoriesObserved: 0 },
          root: { kind: "directory", name: "", children: [{ kind: "file", name: "first.bin", compressedSize: 100, uncompressedSizeDeclared: 200, compressionMethod: "Stored" }] },
          warnings: ["entry_limit"]
        })
      },
      completeness: "partial"
    };
  }
  if (key.includes("hostile")) {
    return {
      providerId: "builtin.archive-zip",
      representation: {
        family: "archive_tree",
        encodedTree: JSON.stringify({
          ...complete,
          progress: { inspectedEntries: 3, state: "complete", limitReason: null },
          totals: { entriesObserved: 3, filesObserved: 3, directoriesObserved: 0, compressedBytesObserved: 9, uncompressedBytesDeclaredObserved: 18 },
          root: {
            kind: "directory",
            name: "",
            children: [
              { kind: "file", name: "../escaped.txt", unsafeName: true, compressedSize: 3, uncompressedSizeDeclared: 6, compressionMethod: "Stored" },
              { kind: "file", name: "C:\\secret.txt", unsafeName: true, compressedSize: 3, uncompressedSizeDeclared: 6, compressionMethod: "Stored" },
              { kind: "file", name: "nested.zip", compressedSize: 3, uncompressedSizeDeclared: 6, compressionMethod: "Deflated" }
            ]
          },
          warnings: ["unsafe_name"]
        })
      },
      completeness: "complete"
    };
  }
  return {
    providerId: "builtin.archive-zip",
    representation: { family: "archive_tree", encodedTree: JSON.stringify(complete) },
    completeness: "complete"
  };
}

function isW308SourceKey(key: string) {
  // W3-10 composes multiple provider fixtures on one page; ZIP truth must remain source-scoped.
  return key.includes("archive") || key.startsWith("w3-08-");
}

function w307Metadata(source: PreviewSnapshot["source"]): PreviewMetadata | null {
  if (!isW307FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  const names: Array<[string, string]> = [
    ["empty", "W3-07 empty folder"],
    ["mixed", "W3-07 mixed folder"],
    ["1000", "W3-07 1,000-entry folder"],
    ["100001", "W3-07 100,001-entry folder"],
    ["10000", "W3-07 10,000-entry folder"],
    ["100000", "W3-07 100,000-entry folder"],
    ["deadline", "W3-07 deadline-bounded folder"]
  ];
  const displayName = names.find(([suffix]) => key.includes(suffix))?.[1];
  return displayName === undefined ? null : {
    displayName,
    mediaType: null,
    extension: null,
    sizeBytes: 0,
    modifiedAtEpochMs: null,
    materialization: "metadata_only",
    readEligibility: "metadata_only"
  };
}

function w306Metadata(source: PreviewSnapshot["source"]): PreviewMetadata | null {
  if (!isW306FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  const definitions: Array<[string, PreviewMetadata]> = [
    ["png", {
      displayName: "W3-06 bounded PNG image",
      mediaType: "image/png",
      extension: "png",
      sizeBytes: 1_024,
      modifiedAtEpochMs: 16,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["jpeg", {
      displayName: "W3-06 bounded JPEG image",
      mediaType: "image/jpeg",
      extension: "jpg",
      sizeBytes: 2_048,
      modifiedAtEpochMs: 17,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["partial", {
      displayName: "W3-06 Partial bounded image",
      mediaType: "image/png",
      extension: "png",
      sizeBytes: 12_582_912,
      modifiedAtEpochMs: 18,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["corrupt", {
      displayName: "W3-06 corrupt image",
      mediaType: "image/png",
      extension: "png",
      sizeBytes: 96,
      modifiedAtEpochMs: 19,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["oversized", {
      displayName: "W3-06 oversized image",
      mediaType: "image/png",
      extension: "png",
      sizeBytes: 1_048_576,
      modifiedAtEpochMs: 20,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["unsupported", {
      displayName: "W3-06 unsupported SVG image",
      mediaType: "image/svg+xml",
      extension: "svg",
      sizeBytes: 4_096,
      modifiedAtEpochMs: 21,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }]
  ];
  return definitions.find(([suffix]) => key.includes(suffix))?.[1] ?? null;
}

function w306ImageDescriptor(source: PreviewSnapshot["source"]): PreviewAssetArtifact | null {
  if (!isW306FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  if (key.includes("corrupt") || key.includes("oversized") || key.includes("unsupported")) return null;
  if (key.includes("jpeg")) {
    return { mediaType: "image/jpeg", bytes: MOCK_JPEG_BYTES };
  }
  if (key.includes("png") || key.includes("partial")) {
    return { mediaType: "image/png", bytes: MOCK_THUMBNAIL_BYTES };
  }
  return null;
}

function w306Representation(source: PreviewSnapshot["source"]):
  (Pick<PreviewRepresentationEnvelope, "representation" | "completeness"> & { providerId: string }) | null {
  if (!isW306FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  const descriptor = w306ImageDescriptor(source);
  if (descriptor === null) return null;
  return {
    providerId: "builtin.image",
    representation: {
      family: "image",
      assetToken: `w306-asset-${key}`,
      mediaType: descriptor.mediaType
    },
    completeness: key.includes("partial") ? "partial" : "complete"
  };
}

function w307Representation(source: PreviewSnapshot["source"]):
  (Pick<PreviewRepresentationEnvelope, "representation" | "completeness"> & { providerId: string }) | null {
  if (!isW307FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  const definition = key.includes("empty")
    ? { folderName: "W3-07 empty folder", total: 0, files: 0, directories: 0, state: "complete" as const, limitReason: null }
    : key.includes("mixed")
    ? { folderName: "W3-07 mixed folder", total: 4, files: 2, directories: 2, state: "complete" as const, limitReason: null }
    : key.includes("1000") && !key.includes("10000") && !key.includes("100000")
    ? { folderName: "W3-07 1,000-entry folder", total: 1_000, files: 800, directories: 200, state: "complete" as const, limitReason: null }
     : key.includes("100001")
     ? { folderName: "W3-07 100,001-entry folder", total: 100_000, files: 90_000, directories: 10_000, state: "partial" as const, limitReason: "entry_limit" as const }
     : key.includes("10000") && !key.includes("100000")
     ? { folderName: "W3-07 10,000-entry folder", total: 10_000, files: 8_000, directories: 2_000, state: "complete" as const, limitReason: null }
     : key.includes("100000")
     ? { folderName: "W3-07 100,000-entry folder", total: 100_000, files: 90_000, directories: 10_000, state: "complete" as const, limitReason: null }
    : key.includes("deadline")
    ? { folderName: "W3-07 deadline-bounded folder", total: 10_000, files: 8_000, directories: 2_000, state: "partial" as const, limitReason: "deadline" as const }
    : null;
  if (definition === null) return null;
  const sample = definition.total === 0
    ? []
    : [
      { name: "README.md", kind: "file", extension: "md", sizeBytes: 1_024 },
      { name: "src", kind: "directory", extension: null, sizeBytes: null },
      { name: "package.json", kind: "file", extension: "json", sizeBytes: 2_048 }
    ];
  const extensionCounts = definition.files === 0
    ? []
    : [
      { extension: "md", count: Math.floor(definition.files * .5) },
      { extension: "txt", count: definition.files - Math.floor(definition.files * .5) }
    ];
  const payload = {
    version: 1,
    folderName: definition.folderName,
    progress: {
      inspectedEntries: definition.total,
      acceptedChildren: definition.total,
      state: definition.state,
      limitReason: definition.limitReason
    },
    sample,
    kindCounts: { files: definition.files, directories: definition.directories, other: 0 },
    extensionCounts,
    sizeProgress: { observedBytes: definition.files * 1_024, knownSizeEntries: definition.files },
    largestObserved: definition.files === 0 ? [] : [{ name: "package.json", sizeBytes: 2_048 }],
    projectHints: definition.files === 0 ? [] : ["Node.js project", "README"]
  };
  return {
    providerId: "builtin.folder",
    representation: { family: "folder_summary", encodedSummary: JSON.stringify(payload) },
    completeness: definition.state === "complete" ? "complete" : "partial"
  };
}

function w307ProgressiveSnapshots(readySnapshot: PreviewSnapshot): [PreviewSnapshot, PreviewSnapshot] | null {
  const envelope = readySnapshot.representation;
  if (envelope?.representation.family !== "folder_summary") return null;
  let finalPayload: FolderSummaryPayloadV1;
  try {
    finalPayload = JSON.parse(envelope.representation.encodedSummary) as FolderSummaryPayloadV1;
  } catch {
    return null;
  }
  if (finalPayload.progress.inspectedEntries < 2) return null;
  const firstInspected = Math.min(1, finalPayload.progress.inspectedEntries);
  const laterInspected = Math.min(
    finalPayload.progress.inspectedEntries,
    Math.max(firstInspected + 1, Math.min(3, finalPayload.progress.inspectedEntries))
  );
  const partialSnapshot = (inspectedEntries: number): PreviewSnapshot => ({
    ...readySnapshot,
    representation: {
      ...envelope,
      completeness: "partial",
      representation: {
        family: "folder_summary",
        encodedSummary: JSON.stringify(progressiveFolderPayload(finalPayload, inspectedEntries))
      }
    }
  });
  return [partialSnapshot(firstInspected), partialSnapshot(laterInspected)];
}

function progressiveFolderPayload(
  finalPayload: FolderSummaryPayloadV1,
  inspectedEntries: number
): FolderSummaryPayloadV1 {
  const files = Math.min(finalPayload.kindCounts.files, inspectedEntries);
  const directories = Math.min(
    finalPayload.kindCounts.directories,
    Math.max(0, inspectedEntries - files)
  );
  const other = Math.min(
    finalPayload.kindCounts.other,
    Math.max(0, inspectedEntries - files - directories)
  );
  const acceptedChildren = files + directories + other;
  let remainingFiles = files;
  const extensionCounts = finalPayload.extensionCounts.flatMap((bucket) => {
    if (remainingFiles === 0) return [];
    const count = Math.min(bucket.count, remainingFiles);
    remainingFiles -= count;
    return count === 0 ? [] : [{ ...bucket, count }];
  });
  const knownSizeEntries = Math.min(finalPayload.sizeProgress.knownSizeEntries, files);
  return {
    ...finalPayload,
    progress: {
      inspectedEntries,
      acceptedChildren,
      state: "partial",
      limitReason: null
    },
    sample: finalPayload.sample.slice(0, Math.min(finalPayload.sample.length, acceptedChildren)),
    kindCounts: { files, directories, other },
    extensionCounts,
    sizeProgress: {
      observedBytes: Math.min(finalPayload.sizeProgress.observedBytes, knownSizeEntries * 2_048),
      knownSizeEntries
    },
    largestObserved: finalPayload.largestObserved.slice(0, Math.min(finalPayload.largestObserved.length, files)),
    projectHints: acceptedChildren === 0 ? [] : finalPayload.projectHints
  };
}

function w305Metadata(source: PreviewSnapshot["source"]): PreviewMetadata | null {
  if (!isW305FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  const definitions: Array<[string, PreviewMetadata]> = [
    ["json", {
      displayName: "W3-05 structured JSON fixture",
      mediaType: "application/json",
      extension: "json",
      sizeBytes: 1_024,
      modifiedAtEpochMs: 6,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["yaml", {
      displayName: "W3-05 structured YAML fixture",
      mediaType: "application/yaml",
      extension: "yaml",
      sizeBytes: 1_024,
      modifiedAtEpochMs: 7,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["xml", {
      displayName: "W3-05 structured XML fixture",
      mediaType: "application/xml",
      extension: "xml",
      sizeBytes: 1_024,
      modifiedAtEpochMs: 8,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["csv", {
      displayName: "W3-05 CSV table fixture",
      mediaType: "text/csv",
      extension: "csv",
      sizeBytes: 1_024,
      modifiedAtEpochMs: 9,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["tsv", {
      displayName: "W3-05 TSV table fixture",
      mediaType: "text/tab-separated-values",
      extension: "tsv",
      sizeBytes: 1_024,
      modifiedAtEpochMs: 10,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["structured-partial", {
      displayName: "W3-05 partial structured fixture",
      mediaType: "application/json",
      extension: "json",
      sizeBytes: 700_000,
      modifiedAtEpochMs: 11,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["table-partial", {
      displayName: "W3-05 partial table fixture",
      mediaType: "text/csv",
      extension: "csv",
      sizeBytes: 700_000,
      modifiedAtEpochMs: 12,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["malformed", {
      displayName: "W3-05 malformed structured fixture",
      mediaType: "application/json",
      extension: "json",
      sizeBytes: 128,
      modifiedAtEpochMs: 13,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }],
    ["fallback", {
      displayName: "W3-05 metadata fallback fixture",
      mediaType: "application/octet-stream",
      extension: "bin",
      sizeBytes: 8_192,
      modifiedAtEpochMs: 14,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    }]
  ];
  return definitions.find(([suffix]) => key.includes(suffix))?.[1] ?? null;
}

function previewSourceKey(source: PreviewSnapshot["source"]) {
  if (source.kind === "managed") return source.fileId.toLowerCase();
  if (source.kind === "ephemeral") return source.entryId.toLowerCase();
  return source.hostToken.toLowerCase();
}

function w304Metadata(source: PreviewSnapshot["source"]): PreviewMetadata | null {
  if (!isW304FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  if (key.includes("readme") || key.includes("markdown")) {
    return {
      displayName: "W3-04 hostile Markdown fixture",
      mediaType: "text/markdown",
      extension: "md",
      sizeBytes: 1_280,
      modifiedAtEpochMs: 3,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    };
  }
  if (key.includes("source") || key.endsWith("-rs")) {
    return {
      displayName: "W3-04 source-code fixture",
      mediaType: "text/x-rust",
      extension: "rs",
      sizeBytes: 768,
      modifiedAtEpochMs: 4,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    };
  }
  if (key.includes("partial")) {
    return {
      displayName: "W3-04 bounded partial fixture",
      mediaType: "text/plain",
      extension: "txt",
      sizeBytes: 700_000,
      modifiedAtEpochMs: 5,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    };
  }
  if (key.includes("fallback")) {
    return {
      displayName: "W3-04 metadata fallback fixture",
      mediaType: "application/octet-stream",
      extension: "bin",
      sizeBytes: 8_192,
      modifiedAtEpochMs: 6,
      materialization: "boundary_readable",
      readEligibility: "eligible"
    };
  }
  return null;
}

function w304Representation(source: PreviewSnapshot["source"]):
  (Pick<PreviewRepresentationEnvelope, "representation" | "completeness"> & { providerId: string }) | null {
  const key = previewSourceKey(source);
  if (key.includes("readme") || key.includes("markdown")) {
    return {
      providerId: "builtin.markdown",
      representation: {
        family: "safe_html",
        html: "<h1>W3-09 rendered Markdown</h1><p><strong>Bold text</strong> is rendered as document content.</p><ul><li>First list item</li><li>Second list item</li></ul><blockquote><p>A rendered blockquote.</p></blockquote><pre><code>const bounded = true;</code></pre><table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody><tr><td>safe</td><td>yes</td></tr></tbody></table><p>Unsafe HTML and resource references were removed before rendering.</p>"
      },
      completeness: "complete"
    };
  }
  if (key.includes("source") || key.endsWith("-rs")) {
    return {
      providerId: "builtin.source-code",
      representation: {
        family: "text",
        text: "fn main() {\n    println!(\"W3-04\");\n}\n",
        language: "rust"
      },
      completeness: "complete"
    };
  }
  if (key.includes("partial")) {
    return {
      providerId: "builtin.text",
      representation: {
        family: "text",
        text: "bounded prefix\n".repeat(512),
        language: null
      },
      completeness: "partial"
    };
  }
  return null;
}

function w305Representation(source: PreviewSnapshot["source"]):
  (Pick<PreviewRepresentationEnvelope, "representation" | "completeness"> & { providerId: string }) | null {
  if (!isW305FixtureEnabled()) return null;
  const key = previewSourceKey(source);
  const noTruncation = { depth: false, nodes: false, strings: false };
  const noTableTruncation = { rows: false, columns: false, cells: false };
  if (key.includes("json")) {
    return {
      providerId: "builtin.structured-json",
      representation: {
        family: "structured_tree",
        encodedTree: JSON.stringify({
          schemaVersion: 1,
          format: "json",
          root: {
            kind: "object",
            entries: [
              { key: "project", value: { kind: "scalar", scalarType: "string", value: "Fylura" } },
              { key: "enabled", value: { kind: "scalar", scalarType: "boolean", value: "true" } },
              { key: "tags", value: { kind: "array", items: [
                { kind: "scalar", scalarType: "string", value: "preview" },
                { kind: "scalar", scalarType: "string", value: "bounded" }
              ] } }
            ]
          },
          truncation: noTruncation
        })
      },
      completeness: "complete"
    };
  }
  if (key.includes("yaml")) {
    return {
      providerId: "builtin.structured-yaml",
      representation: {
        family: "structured_tree",
        encodedTree: JSON.stringify({
          schemaVersion: 1,
          format: "yaml",
          root: {
            kind: "object",
            entries: [
              { key: "service", value: { kind: "scalar", scalarType: "string", value: "preview" } },
              { key: "limits", value: { kind: "object", entries: [
                { key: "rows", value: { kind: "scalar", scalarType: "number", value: "500" } },
                { key: "aliases", value: { kind: "scalar", scalarType: "string", value: "inert" } }
              ] } }
            ]
          },
          truncation: noTruncation
        })
      },
      completeness: "complete"
    };
  }
  if (key.includes("xml")) {
    return {
      providerId: "builtin.structured-xml",
      representation: {
        family: "structured_tree",
        encodedTree: JSON.stringify({
          schemaVersion: 1,
          format: "xml",
          root: {
            kind: "element",
            name: "root",
            attributes: [{ name: "data-kind", value: "safe" }],
            children: [
              { kind: "element", name: "message", attributes: [], children: [
                { kind: "text", value: "<script>inert text</script>" }
              ] },
              { kind: "text", value: "https://example.invalid/remote" }
            ]
          },
          truncation: noTruncation
        })
      },
      completeness: "complete"
    };
  }
  if (key.includes("csv")) {
    return {
      providerId: "builtin.table-csv",
      representation: {
        family: "table",
        encodedTable: JSON.stringify({
          schemaVersion: 1,
          format: "csv",
          columns: ["Name", "Value"],
          rows: [["alpha", "=SUM(A1:A2)"], ["beta", "quoted, cell"], ["gamma", "+1+1"], ["delta", "@COMMAND"]],
          truncation: noTableTruncation
        })
      },
      completeness: "complete"
    };
  }
  if (key.includes("tsv")) {
    return {
      providerId: "builtin.table-tsv",
      representation: {
        family: "table",
        encodedTable: JSON.stringify({
          schemaVersion: 1,
          format: "tsv",
          columns: ["Name", "Value"],
          rows: [["one", "1"], ["ragged", "2", "extra"]],
          truncation: noTableTruncation
        })
      },
      completeness: "complete"
    };
  }
  if (key.includes("structured-partial")) {
    return {
      providerId: "builtin.structured-json",
      representation: {
        family: "structured_tree",
        encodedTree: JSON.stringify({
          schemaVersion: 1,
          format: "json",
          root: { kind: "object", entries: [{ key: "loaded", value: { kind: "scalar", scalarType: "boolean", value: "true" } }] },
          truncation: { depth: false, nodes: true, strings: false }
        })
      },
      completeness: "partial"
    };
  }
  if (key.includes("table-partial")) {
    return {
      providerId: "builtin.table-csv",
      representation: {
        family: "table",
        encodedTable: JSON.stringify({
          schemaVersion: 1,
          format: "csv",
          columns: ["Name", "Value"],
          rows: [["loaded", "=literal"]],
          truncation: { rows: true, columns: false, cells: false }
        })
      },
      completeness: "partial"
    };
  }
  return null;
}

function metadataCapabilities() {
  return {
    canSearch: false,
    canZoom: false,
    canPlayback: false,
    canSelectText: false,
    canNavigateInternal: false,
    canNavigateSiblings: false,
    canOpenExternal: true,
    canReveal: true,
    canRequestMaterialization: false
  };
}

function getSession(sessionId: string) {
  const session = sessions.get(sessionId);
  if (!session || session.disposed) throw new Error("browse_session_not_found");
  return session;
}

function getPreview(previewId: string) {
  const record = previews.get(previewId);
  if (!record) throw new Error("preview_session_not_found");
  return record;
}

function id(prefix: string) {
  nextId += 1;
  return `mock-${prefix}-${nextId}`;
}

/** Browser fixture only: no native identity or filesystem acceptance claim. */
export function mockSearchParent(entry: GlobalSearchResult): BrowseOpenResponse {
  const parent = entry.path.replace(/[/\\][^/\\]+$/, "");
  const response = newBrowseSession(parent, true);
  const session = getSession(response.sessionId);
  const enumerationId = `${response.sessionId}-search-enumeration`;
  const ref = { kind: "ephemeral" as const, browseSessionId: response.sessionId, entryId: `${enumerationId}-target` };
  const pathRef = entry.isDirectory ? { id: `${enumerationId}-directory` } : undefined;
  const page: BrowsePage = {
    sessionId: response.sessionId, requestId: "search-admission", enumerationId,
    entries: [{ ref, pathRef, name: entry.name, displayPath: entry.path,
      kind: entry.isDirectory ? "directory" : "file", materialization: "unknown" }],
    completion: "complete", knownCount: 1
  };
  session.entries.set(ref.entryId, { enumerationId, ...(pathRef ? { pathRefId: pathRef.id } : {}) });
  if (pathRef) session.pathRefs.add(pathRef.id);
  session.enumeration = { requestId: page.requestId, enumerationId, generation: 1,
    query: { text: null, entryKind: "all" }, pathRefId: response.rootPathRef.id, nextIndex: 1 };
  response.initialPage = page;
  // Existing browser index fixtures have only path identities, hence no selection.
  return response;
}

const managementEntries = new Map<string, BrowseEntry>();
export function resolveManagementMockEntry(ref: import("../types/fileWorkspace").EntryRef) {
  if (ref.kind !== "ephemeral") throw Error("browse_entry_ref_invalid");
  const session = getSession(ref.browseSessionId);
  const entry = managementEntries.get(ref.entryId);
  if (!entry || !session.entries.has(ref.entryId)) throw Error("browse_entry_ref_invalid");
  return { name: entry.name, path: entry.displayPath, isDirectory: entry.kind === "directory" };
}

export function resolveManagementMockLocation(sessionId: string) {
  const session = getSession(sessionId);
  return session.routingHint ?? session.location.displayName;
}
