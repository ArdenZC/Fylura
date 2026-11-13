// @vitest-environment happy-dom

import { act, createElement, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FileWorkspaceApi } from "../src/api/fileWorkspaceApi";
import { FileWorkspaceController } from "../src/fileWorkspace";
import { makeTranslator } from "../src/i18n";
import type { FileLibrarySummary } from "../src/types/domain";
import type {
  BrowseEntry,
  BrowsePage,
  PreviewCapabilities,
  PreviewHostKind,
  PreviewSnapshot,
  PreviewSourceRef
} from "../src/types/fileWorkspace";
import { adaptBrowseEntry, adaptLibraryCollection, adaptLibrarySummary } from "../src/views/fileLibrary/presentation/adapters";
import {
  createBrowseInteractionProjection,
  createLibraryInteractionProjection
} from "../src/views/fileLibrary/list/interactionAdapters";
import type { PresentationInteractionProjection } from "../src/views/fileLibrary/list/interactionContracts";
import type { PresentationEntry } from "../src/views/fileLibrary/presentation/contracts";
import { SharedFileGrid } from "../src/views/fileLibrary/list/SharedFileGrid";
import { SharedFileList } from "../src/views/fileLibrary/list/SharedFileList";
import type { BrowseSourceOwner } from "../src/views/fileLibrary/browse/browseSourceOwner";
import type { LibrarySourceOwner } from "../src/views/fileLibrary/library/librarySourceOwner";
import {
  handleFloatingPreviewSpace,
  isFloatingPreviewCloseSpaceEligible,
  isPreviewWorkspaceSpaceEligible,
  PreviewExperienceController,
  previewPhaseForBackendError,
  type FloatingPreviewSpaceEvent,
  type PreviewSpaceEvent
} from "../src/views/fileLibrary/preview/previewExperienceController";
import { previewSourceFromEntry } from "../src/views/fileLibrary/preview/previewSource";

const t = makeTranslator("en");
const nativeClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const nativeClientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
const nativeOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
const capabilities: PreviewCapabilities = {
  canSearch: false,
  canZoom: false,
  canPlayback: false,
  canSelectText: false,
  canNavigateInternal: false,
  canNavigateSiblings: false,
  canOpenExternal: false,
  canReveal: false,
  canRequestMaterialization: false
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function emptyPage(): BrowsePage {
  return {
    sessionId: "session",
    requestId: "request",
    enumerationId: "enumeration",
    entries: [],
    completion: "complete"
  };
}

function makeSnapshot(
  previewId: string,
  requestId: string,
  source: PreviewSnapshot["source"],
  state: PreviewSnapshot["state"] = "idle",
  hostKind: PreviewHostKind = "zen_floating"
): PreviewSnapshot {
  return {
    previewId,
    sessionId: previewId,
    requestId,
    source,
    hostKind,
    state,
    effectiveCapabilities: capabilities
  };
}

function makeNativeSnapshot(
  previewId: string,
  requestId: string,
  source: PreviewSnapshot["source"],
  sourceVersion: string,
  token: string
): PreviewSnapshot {
  return {
    ...makeSnapshot(previewId, requestId, source, "ready"),
    sourceVersion,
    representation: {
      sourceVersion,
      representation: { family: "native_opaque", host: "zen_floating", token },
      completeness: "complete",
      warnings: [],
      capabilities
    },
    activeProviderId: "native.macos.quick-look"
  };
}

function makeMetadataFallbackSnapshot(
  previewId: string,
  requestId: string,
  source: PreviewSnapshot["source"],
  sourceVersion: string
): PreviewSnapshot {
  return {
    ...makeSnapshot(previewId, requestId, source, "ready"),
    sourceVersion,
    representation: {
      sourceVersion,
      representation: {
        family: "metadata",
        metadata: {
          displayName: "native-fallback.pdf",
          mediaType: "application/pdf",
          extension: "pdf",
          sizeBytes: 128,
          modifiedAtEpochMs: 1,
          materialization: "local",
          readEligibility: "eligible"
        }
      },
      completeness: "complete",
      warnings: [
        { kind: "provider_fallback", providerId: "native.macos.quick-look", reason: "failed" },
        { kind: "metadata_fallback" }
      ],
      capabilities
    },
    activeProviderId: undefined
  };
}

function makeProgressiveFolderSnapshot(
  previewId: string,
  requestId: string,
  source: PreviewSnapshot["source"],
  inspectedEntries: number,
  completeness: "partial" | "complete" = "partial"
): PreviewSnapshot {
  const snapshot = makeSnapshot(previewId, requestId, source, "ready");
  return {
    ...snapshot,
    sourceVersion: "folder-version",
    representation: {
      sourceVersion: "folder-version",
      representation: { family: "folder_summary", encodedSummary: `folder-${inspectedEntries}` },
      completeness,
      warnings: [],
      capabilities
    },
    effectiveCapabilities: capabilities,
    activeProviderId: "builtin.folder"
  };
}

function makeTerminalMetadataSnapshot(
  previewId: string,
  requestId: string,
  source: PreviewSnapshot["source"],
  condition: "source_unavailable" | "materialization_required" | "permission_denied" | "identity_changed" | "cancelled"
): PreviewSnapshot {
  const snapshot = makeSnapshot(previewId, requestId, source, "failed");
  return {
    ...snapshot,
    sourceVersion: "terminal-version",
    representation: {
      sourceVersion: "terminal-version",
      representation: {
        family: "metadata",
        metadata: {
          displayName: "terminal-fixture.txt",
          mediaType: "text/plain",
          extension: "txt",
          sizeBytes: 4_096,
          modifiedAtEpochMs: 1,
          materialization: "remote_placeholder",
          readEligibility: "materialization_required"
        }
      },
      completeness: "complete",
      warnings: [{ kind: "terminal_condition", condition }],
      capabilities
    },
    effectiveCapabilities: capabilities,
    activeProviderId: "builtin.terminal"
  };
}

function makePreviewApi({
  deferCreate = false,
  deferSwitch = false,
  deferSnapshots = false,
  startError
}: { deferCreate?: boolean; deferSwitch?: boolean; deferSnapshots?: boolean; startError?: string } = {}) {
  const records = new Map<string, PreviewSnapshot>();
  let nextPreviewId = 0;
  const creates: Array<{
    snapshot: PreviewSnapshot;
    deferred: ReturnType<typeof deferred<PreviewSnapshot>>;
  }> = [];
  const starts: Array<{
    previewId: string;
    requestId: string;
    source: PreviewSourceRef;
    hostKind: PreviewHostKind;
    deferred: ReturnType<typeof deferred<PreviewSnapshot>>;
  }> = [];
  const switches: Array<{
    previewId: string;
    requestId: string;
    source: PreviewSourceRef;
    hostKind: PreviewHostKind;
    deferred: ReturnType<typeof deferred<PreviewSnapshot>>;
  }> = [];
  const snapshots: Array<{
    previewId: string;
    deferred: ReturnType<typeof deferred<PreviewSnapshot>>;
  }> = [];
  const previewCancel = vi.fn(async () => true);
  const previewDispose = vi.fn(async ({ previewId }: { previewId: string }) => {
    records.delete(previewId);
    return true;
  });
  const api: FileWorkspaceApi = {
    browseOpen: async () => { throw new Error("unused"); },
    browseRestore: async () => { throw new Error("unused"); },
    locationBrowse: async () => { throw new Error("unused"); },
    browseStartEnumeration: async () => emptyPage(),
    browseNextPage: async () => emptyPage(),
    browseCancel: async () => undefined,
    browseReleasePage: async () => undefined,
    browseReleasePath: async () => undefined,
    browseRetainPath: async () => undefined,
    browseDispose: async () => undefined,
    locationList: async () => [],
    changeStart: async () => ({ monitorId: "monitor", sessionId: "session", pathRef: { id: "path" } }),
    changePending: async () => null,
    changeRefresh: async () => emptyPage(),
    changeDispose: async () => undefined,
    readEligibility: async ({ source }) => ({ source, eligibility: "eligible" }),
    thumbnailRequest: async () => ({ cacheKey: "cache", bytes: new Uint8Array() }),
    thumbnailCancel: async () => true,
    previewCreate: async ({ requestId, source, hostKind }) => {
      const snapshot = makeSnapshot(`preview-${++nextPreviewId}`, requestId, source, "idle", hostKind);
      if (deferCreate) {
        const pending = deferred<PreviewSnapshot>();
        creates.push({ snapshot, deferred: pending });
        return pending.promise;
      }
      records.set(snapshot.previewId, snapshot);
      return snapshot;
    },
    previewSnapshot: async ({ previewId }) => {
      const snapshot = records.get(previewId) ?? makeSnapshot(previewId, "missing", { kind: "managed", fileId: "missing" });
      if (!deferSnapshots) return snapshot;
      const pending = deferred<PreviewSnapshot>();
      snapshots.push({ previewId, deferred: pending });
      return pending.promise;
    },
    previewStart: async ({ previewId }) => {
      const snapshot = records.get(previewId);
      if (!snapshot) throw new Error("preview_missing");
      if (startError !== undefined) throw new Error(startError);
      const pending = deferred<PreviewSnapshot>();
      starts.push({
        previewId,
        requestId: snapshot.requestId,
        source: snapshot.source,
        hostKind: snapshot.hostKind,
        deferred: pending
      });
      return pending.promise;
    },
    previewCancel,
    previewDispose,
    previewSwitchSource: async ({ previewId, requestId, source }) => {
      const snapshot = records.get(previewId);
      if (!snapshot) throw new Error("preview_missing");
      const next = makeSnapshot(previewId, requestId, source, "resolving");
      if (deferSwitch) {
        const pending = deferred<PreviewSnapshot>();
        switches.push({ previewId, requestId, source, hostKind: snapshot.hostKind, deferred: pending });
        return pending.promise;
      }
      records.set(previewId, next);
      return next;
    },
    previewAssetRequest: async () => { throw new Error("unused"); }
  };
  function resolveSwitch(pending: (typeof switches)[number]) {
    const next = makeSnapshot(pending.previewId, pending.requestId, pending.source, "resolving", pending.hostKind);
    records.set(pending.previewId, next);
    pending.deferred.resolve(next);
  }

  function rejectSwitch(pending: (typeof switches)[number], error: string) {
    pending.deferred.reject(new Error(error));
  }

  function resolveCreate(pending: (typeof creates)[number]) {
    records.set(pending.snapshot.previewId, pending.snapshot);
    pending.deferred.resolve(pending.snapshot);
  }

  function resolveStart(
    pending: (typeof starts)[number],
    nextSnapshot = makeSnapshot(pending.previewId, pending.requestId, pending.source, "ready", pending.hostKind)
  ) {
    pending.deferred.resolve(nextSnapshot);
  }

  function getBackendPreview(previewId: string) {
    return records.get(previewId);
  }

  function rejectStart(pending: (typeof starts)[number], error: string) {
    pending.deferred.reject(new Error(error));
  }

  return { api, creates, starts, switches, snapshots, resolveCreate, resolveSwitch, rejectSwitch, resolveStart, rejectStart, getBackendPreview, previewCancel, previewDispose };
}

function summary(id: string): FileLibrarySummary {
  return {
    id,
    name: `${id}.txt`,
    extension: "txt",
    displayDirectory: "Documents",
    size: 12,
    modifiedAt: 1,
    createdAt: 1,
    isDirectory: false,
    fileType: "text",
    purpose: "document",
    lifecycle: "active",
    risk: "low",
    confidence: 1,
    isDuplicate: false,
    requiresReview: false,
    isStale: false,
    tags: [],
    tagCount: 0
  };
}

function source(id: string) {
  const entry = adaptLibrarySummary(summary(id));
  return previewSourceFromEntry(entry, adaptLibraryCollection({ queryFingerprint: "query", snapshotRevision: 1 }));
}

async function flush() {
  for (let index = 0; index < 6; index += 1) await Promise.resolve();
}

function browseEntry(id: string): BrowseEntry {
  return {
    ref: { kind: "ephemeral", browseSessionId: "browse-session", entryId: id },
    name: `${id}.md`,
    displayPath: `${id}.md`,
    kind: "file",
    materialization: "unknown"
  };
}

function keyboardInteractionModel(source: "library" | "browse") {
  const focusedId = { value: null as string | null };
  const focusCalls = vi.fn();
  const selectionCalls = {
    select: vi.fn(),
    selectAll: vi.fn(),
    clearSelection: vi.fn(),
    toggle: vi.fn()
  };
  const libraryFiles = [summary("library-a"), summary("library-b")];
  const entries = source === "library"
    ? libraryFiles.map((file) => adaptLibrarySummary(file))
    : [adaptBrowseEntry(browseEntry("browse-a")), adaptBrowseEntry(browseEntry("browse-b"))];
  const collection = source === "library"
    ? adaptLibraryCollection({ queryFingerprint: "keyboard-query", snapshotRevision: 7 })
    : {
      source: "browse" as const,
      provenance: {
        sessionId: "browse-session",
        requestId: "keyboard-request",
        enumerationId: "keyboard-enumeration",
        completion: "complete" as const,
        knownCount: entries.length
      }
    };

  if (source === "library") {
    const owner = {
      files: libraryFiles,
      totalCount: entries.length,
      collection,
      get focusedId() { return focusedId.value; },
      selection: null,
      selectionContainsFileId: () => false,
      hasMore: false,
      isLoading: false,
      loadNextPage: async () => undefined,
      setExplicitSelection: selectionCalls.select,
      setFocusedId: vi.fn((id: string | null) => {
        focusedId.value = id;
        focusCalls(id);
      }),
      toggleSelection: selectionCalls.toggle,
      selectAllMatching: selectionCalls.selectAll,
      clearSelection: selectionCalls.clearSelection
    } as unknown as LibrarySourceOwner;
    return {
      entries,
      collection,
      focusedId,
      focusCalls,
      selectionCalls,
      projection: () => createLibraryInteractionProjection(owner)
    };
  }

  const browseEntries = entries as ReturnType<typeof adaptBrowseEntry>[];
  const owner = {
    entries: browseEntries,
    collection,
    get focusedId() { return focusedId.value; },
    selectedIds: new Set<string>(),
    hasMore: false,
    enumerationState: "complete",
    loadNextPage: async () => undefined,
    selectEntry: selectionCalls.select,
    selectAllLoaded: selectionCalls.selectAll,
    selectAllDirectory: selectionCalls.selectAll,
    clearSelection: selectionCalls.clearSelection,
    setFocusedId: vi.fn((id: string | null) => {
      focusedId.value = id;
      focusCalls(id);
    })
  } as unknown as BrowseSourceOwner;
  return {
    entries,
    collection,
    focusedId,
    focusCalls,
    selectionCalls,
    projection: () => createBrowseInteractionProjection(owner)
  };
}

function previewControllerStub() {
  return {
    requestThumbnail: vi.fn(async () => null),
    cancelThumbnail: vi.fn(async () => undefined)
  } as unknown as FileWorkspaceController;
}

async function mountPreviewSurface(
  kind: "list" | "grid",
  interaction: PresentationInteractionProjection,
  onPreview: (entry: PresentationEntry, trigger: HTMLElement, event: ReactKeyboardEvent<HTMLDivElement>) => boolean
) {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, value: 640 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, value: 240 });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, value: 240 });
  const container = document.body.appendChild(document.createElement("div"));
  const root = createRoot(container);
  const render = async (nextInteraction: PresentationInteractionProjection) => {
    await act(async () => {
      if (kind === "list") {
        root.render(createElement(SharedFileList, {
          interaction: nextInteraction,
          language: "en",
          t,
          ariaLabel: "Files",
          onPreview
        }));
      } else {
        root.render(createElement(SharedFileGrid, {
          interaction: nextInteraction,
          language: "en",
          t,
          controller: previewControllerStub(),
          ariaLabel: "Files",
          onPreview
        }));
      }
      await Promise.resolve();
    });
  };
  await render(interaction);
  const surface = container.querySelector<HTMLElement>(kind === "list" ? "[role='listbox']" : "[role='grid']");
  if (surface === null) throw new Error(`SharedFile${kind === "list" ? "List" : "Grid"} did not mount`);
  return { container, root, render, surface };
}

const previewSurfaceCases = [
  ["Library List", "library", "list"],
  ["Library Grid", "library", "grid"],
  ["Browse List", "browse", "list"],
  ["Browse Grid", "browse", "grid"]
] as const;

describe("W3-02 Zen floating quick preview", () => {
  it("keeps source refs opaque and bounded to the loaded collection", () => {
    const library = source("file-a");
    expect(library?.previewSource).toEqual({ kind: "managed", fileId: "file-a" });
    expect(JSON.stringify(library)).not.toContain("path");

    const browseEntry = adaptBrowseEntry({
      ref: { kind: "ephemeral", browseSessionId: "browse-1", entryId: "entry-1" },
      pathRef: { id: "opaque-folder-ref" },
      name: "notes.md",
      displayPath: "not-an-authority",
      kind: "file",
      materialization: "unknown"
    });
    const browse = previewSourceFromEntry(browseEntry, {
      source: "browse",
      provenance: {
        sessionId: "browse-1",
        requestId: "request-1",
        enumerationId: "enumeration-1",
        completion: "complete",
        knownCount: 1
      }
    });
    expect(browse?.previewSource).toEqual({ kind: "ephemeral", browseSessionId: "browse-1", entryId: "entry-1" });
    expect(JSON.stringify(browse)).not.toContain("opaque-folder-ref");
  });

  it("reproduces stale cross-preview-id publication for one floating host", async () => {
    const { api, creates, resolveCreate, previewDispose } = makePreviewApi({ deferCreate: true });
    const workspace = new FileWorkspaceController(api);
    const first = source("file-a")!;
    const second = source("file-b")!;
    const third = source("file-c")!;

    const firstRequest = workspace.createPreview({
      requestId: "request-a",
      source: first.previewSource,
      hostKind: "zen_floating"
    });
    const secondRequest = workspace.createPreview({
      requestId: "request-b",
      source: second.previewSource,
      hostKind: "zen_floating"
    });
    const thirdRequest = workspace.createPreview({
      requestId: "request-c",
      source: third.previewSource,
      hostKind: "zen_floating"
    });
    expect(creates).toHaveLength(3);

    resolveCreate(creates[2]!);
    await expect(thirdRequest).resolves.toEqual(expect.objectContaining({
      previewId: "preview-3",
      requestId: "request-c",
      source: third.previewSource
    }));

    resolveCreate(creates[0]!);
    resolveCreate(creates[1]!);
    await expect(firstRequest).resolves.toBeNull();
    await expect(secondRequest).resolves.toBeNull();
    expect(Object.keys(workspace.getState().previews)).toEqual(["preview-3"]);
    await flush();
    expect(previewDispose).toHaveBeenCalledWith({ previewId: "preview-1" });
    expect(previewDispose).toHaveBeenCalledWith({ previewId: "preview-2" });
  });

  it.each([
    ["A resolves first", [0, 1]],
    ["B resolves first", [1, 0]]
  ] as const)("distinguishes same-tuple create operations when %s", async (_label, resolutionOrder) => {
    const fixture = makePreviewApi({ deferCreate: true });
    const workspace = new FileWorkspaceController(fixture.api);
    const current = source("same-tuple")!;

    const firstRequest = workspace.createPreview({
      requestId: "same",
      source: current.previewSource,
      hostKind: "zen_floating"
    });
    const secondRequest = workspace.createPreview({
      requestId: "same",
      source: current.previewSource,
      hostKind: "zen_floating"
    });
    expect(fixture.creates).toHaveLength(2);

    fixture.resolveCreate(fixture.creates[resolutionOrder[0]]!);
    fixture.resolveCreate(fixture.creates[resolutionOrder[1]]!);

    const [firstSnapshot, secondSnapshot] = await Promise.all([firstRequest, secondRequest]);
    expect(firstSnapshot).toBeNull();
    expect(secondSnapshot).toEqual(expect.objectContaining({
      previewId: "preview-2",
      requestId: "same",
      source: current.previewSource,
      hostKind: "zen_floating"
    }));
    expect(Object.keys(workspace.getState().previews)).toEqual(["preview-2"]);
    await flush();
    expect(fixture.previewDispose).toHaveBeenCalledWith({ previewId: "preview-1" });
  });

  it("supersedes a settled same-tuple preview when a later create begins", async () => {
    const fixture = makePreviewApi();
    const workspace = new FileWorkspaceController(fixture.api);
    const current = source("settled-same-tuple")!;

    const firstSnapshot = await workspace.createPreview({
      requestId: "same",
      source: current.previewSource,
      hostKind: "zen_floating"
    });
    const secondSnapshot = await workspace.createPreview({
      requestId: "same",
      source: current.previewSource,
      hostKind: "zen_floating"
    });

    expect(firstSnapshot?.previewId).toBe("preview-1");
    expect(secondSnapshot?.previewId).toBe("preview-2");
    expect(Object.keys(workspace.getState().previews)).toEqual(["preview-2"]);
    await flush();
    expect(fixture.previewDispose).toHaveBeenCalledWith({ previewId: "preview-1" });
  });

  it("rejects a stale snapshot from an older preview id after a newer host create", async () => {
    const fixture = makePreviewApi({ deferSnapshots: true });
    const workspace = new FileWorkspaceController(fixture.api);
    const first = source("file-a")!;
    const second = source("file-b")!;

    const firstSnapshot = await workspace.createPreview({
      requestId: "request-a",
      source: first.previewSource,
      hostKind: "zen_floating"
    });
    expect(firstSnapshot?.previewId).toBe("preview-1");

    const staleSnapshot = workspace.snapshotPreview(firstSnapshot!.previewId);
    expect(fixture.snapshots).toHaveLength(1);

    const secondSnapshot = await workspace.createPreview({
      requestId: "request-b",
      source: second.previewSource,
      hostKind: "zen_floating"
    });
    expect(secondSnapshot?.previewId).toBe("preview-2");

    fixture.snapshots[0]!.deferred.resolve(
      makeSnapshot("preview-1", firstSnapshot!.requestId, first.previewSource, "ready", "zen_floating")
    );
    await expect(staleSnapshot).resolves.toBeNull();
    await flush();

    expect(Object.keys(workspace.getState().previews)).toEqual(["preview-2"]);
    expect(workspace.getState().previews["preview-2"]?.source).toEqual(second.previewSource);
    expect(fixture.previewDispose).toHaveBeenCalledWith({ previewId: "preview-1" });
  });

  it("rejects a late start from an older preview id after a newer host create", async () => {
    const fixture = makePreviewApi();
    const workspace = new FileWorkspaceController(fixture.api);
    const first = source("file-a")!;
    const second = source("file-b")!;

    const firstSnapshot = await workspace.createPreview({
      requestId: "request-a",
      source: first.previewSource,
      hostKind: "zen_floating"
    });
    const staleStart = workspace.startPreview(firstSnapshot!.previewId);
    expect(fixture.starts).toHaveLength(1);

    const secondSnapshot = await workspace.createPreview({
      requestId: "request-b",
      source: second.previewSource,
      hostKind: "zen_floating"
    });
    expect(secondSnapshot?.previewId).toBe("preview-2");

    fixture.resolveStart(fixture.starts[0]!);
    await expect(staleStart).resolves.toBeNull();
    expect(Object.keys(workspace.getState().previews)).toEqual(["preview-2"]);
    expect(fixture.previewDispose).toHaveBeenCalledWith({ previewId: "preview-1" });
  });

  it("keeps floating and pinned host publications independent", async () => {
    const fixture = makePreviewApi({ deferCreate: true });
    const workspace = new FileWorkspaceController(fixture.api);
    const current = source("same-host-tuple")!;

    const floatingRequest = workspace.createPreview({
      requestId: "same",
      source: current.previewSource,
      hostKind: "zen_floating"
    });
    const pinnedRequest = workspace.createPreview({
      requestId: "same",
      source: current.previewSource,
      hostKind: "zen_pinned"
    });
    expect(fixture.creates).toHaveLength(2);

    fixture.resolveCreate(fixture.creates[1]!);
    await expect(pinnedRequest).resolves.toEqual(expect.objectContaining({
      previewId: "preview-2",
      hostKind: "zen_pinned",
      source: current.previewSource
    }));
    fixture.resolveCreate(fixture.creates[0]!);
    await expect(floatingRequest).resolves.toEqual(expect.objectContaining({
      previewId: "preview-1",
      hostKind: "zen_floating",
      source: current.previewSource
    }));

    expect(workspace.getState().previews["preview-1"]?.hostKind).toBe("zen_floating");
    expect(workspace.getState().previews["preview-2"]?.hostKind).toBe("zen_pinned");
  });

  it("keeps same-tuple switch operations ordered by host generation", async () => {
    const fixture = makePreviewApi({ deferSwitch: true });
    const workspace = new FileWorkspaceController(fixture.api);
    const initial = source("switch-initial")!;
    const target = source("switch-target")!;
    const created = await workspace.createPreview({
      requestId: "initial",
      source: initial.previewSource,
      hostKind: "zen_floating"
    });

    const firstSwitch = workspace.switchPreviewSource({
      previewId: created!.previewId,
      requestId: "same",
      source: target.previewSource
    });
    await flush();
    expect(fixture.switches).toHaveLength(1);

    const secondSwitch = workspace.switchPreviewSource({
      previewId: created!.previewId,
      requestId: "same",
      source: target.previewSource
    });
    await flush();
    expect(fixture.switches).toHaveLength(1);

    fixture.resolveSwitch(fixture.switches[0]!);
    await flush();
    expect(fixture.switches).toHaveLength(2);

    fixture.resolveSwitch(fixture.switches[1]!);
    await expect(firstSwitch).resolves.toBeNull();
    await expect(secondSwitch).resolves.toEqual(expect.objectContaining({
      previewId: created!.previewId,
      requestId: "same",
      source: target.previewSource,
      hostKind: "zen_floating"
    }));
    expect(Object.keys(workspace.getState().previews)).toEqual([created!.previewId]);
    expect(workspace.getState().previews[created!.previewId]?.source).toEqual(target.previewSource);
  });

  it("does not let a late switch restore steal a newer same-tuple host create", async () => {
    const fixture = makePreviewApi({ deferCreate: true, deferSwitch: true });
    const workspace = new FileWorkspaceController(fixture.api);
    const current = source("restore-initial")!;
    const target = source("restore-target")!;

    const initialRequest = workspace.createPreview({
      requestId: "same",
      source: current.previewSource,
      hostKind: "zen_floating"
    });
    await flush();
    fixture.resolveCreate(fixture.creates[0]!);
    const initial = await initialRequest;
    expect(initial?.previewId).toBe("preview-1");

    const staleSwitch = workspace.switchPreviewSource({
      previewId: initial!.previewId,
      requestId: "same",
      source: target.previewSource
    });
    await flush();
    expect(fixture.switches).toHaveLength(1);

    const newerCreate = workspace.createPreview({
      requestId: "same",
      source: target.previewSource,
      hostKind: "zen_floating"
    });
    await flush();
    expect(fixture.creates).toHaveLength(2);

    fixture.rejectSwitch(fixture.switches[0]!, "stale-switch");
    await expect(staleSwitch).resolves.toBeNull();
    fixture.resolveCreate(fixture.creates[1]!);
    await expect(newerCreate).resolves.toEqual(expect.objectContaining({
      previewId: "preview-2",
      requestId: "same",
      source: target.previewSource,
      hostKind: "zen_floating"
    }));

    await flush();
    expect(Object.keys(workspace.getState().previews)).toEqual(["preview-2"]);
    expect(fixture.previewDispose).toHaveBeenCalledWith({ previewId: "preview-1" });
  });

  describe.each(previewSurfaceCases)('%s', (_surfaceLabel, sourceKind, surfaceKind) => {
    it("Space is a no-op when no item is focused", async () => {
      const model = keyboardInteractionModel(sourceKind);
      const onPreview = vi.fn((_entry: PresentationEntry) => true);
      const mounted = await mountPreviewSurface(surfaceKind, model.projection(), onPreview);
      try {
        expect(model.projection().focusedIndex).toBe(-1);

        mounted.surface.focus();
        const noFocusSpace = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true });
        mounted.surface.dispatchEvent(noFocusSpace);

        expect(onPreview).not.toHaveBeenCalled();
        expect(noFocusSpace.defaultPrevented).toBe(false);
        expect(model.focusedId.value).toBeNull();
        expect(model.focusCalls).not.toHaveBeenCalled();
        expect(model.selectionCalls.select).not.toHaveBeenCalled();
        expect(model.selectionCalls.selectAll).not.toHaveBeenCalled();
        expect(model.selectionCalls.clearSelection).not.toHaveBeenCalled();
        expect(model.selectionCalls.toggle).not.toHaveBeenCalled();
      } finally {
        mounted.root.unmount();
        mounted.container.remove();
      }
    });

    it("ArrowDown establishes real focus and Space previews the exact focused item", async () => {
      const model = keyboardInteractionModel(sourceKind);
      const previewTargets: Array<ReturnType<typeof previewSourceFromEntry>> = [];
      const onPreview = vi.fn((entry: PresentationEntry) => {
        previewTargets.push(previewSourceFromEntry(entry, model.projection().collection));
        return true;
      });
      const mounted = await mountPreviewSurface(surfaceKind, model.projection(), onPreview);
      try {
        mounted.surface.focus();
        const arrowDown = new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true });
        mounted.surface.dispatchEvent(arrowDown);
        expect(model.focusCalls).toHaveBeenCalledWith(model.entries[0]!.source === "library"
          ? model.entries[0]!.entryRef.fileId
          : model.entries[0]!.entryRef.entryId);

        await mounted.render(model.projection());
        expect(model.projection().focusedIndex).toBe(0);
        const focusedSpace = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true });
        mounted.surface.dispatchEvent(focusedSpace);

        const expectedEntry = model.entries[0]!;
        expect(onPreview).toHaveBeenCalledTimes(1);
        expect(onPreview.mock.calls[0]?.[0]).toEqual(expectedEntry);
        expect(previewTargets).toEqual([expect.objectContaining({
          source: sourceKind,
          previewSource: expectedEntry.entryRef,
          displayName: expectedEntry.displayName
        })]);
        expect(focusedSpace.defaultPrevented).toBe(true);
      } finally {
        mounted.root.unmount();
        mounted.container.remove();
      }
    });
  });

  it("opens shell-first, invalidates stale A/B results, and publishes only C", async () => {
    const { api, starts } = makePreviewApi();
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const first = source("file-a")!;
    const second = source("file-b")!;
    const third = source("file-c")!;
    const trigger = document.body.appendChild(document.createElement("button"));
    const snapshots: Array<PreviewSnapshot | null> = [];
    controller.subscribe((state) => snapshots.push(state.snapshot));

    expect(controller.open(first, trigger)).toBe(true);
    expect(controller.getState().visible).toBe(true);
    expect(controller.getState().phase).toBe("resolving");
    expect(controller.getState().snapshot).toBeNull();

    await flush();
    expect(starts).toHaveLength(1);
    expect(controller.open(second, trigger)).toBe(true);
    await flush();
    expect(starts).toHaveLength(2);
    expect(controller.open(third, trigger)).toBe(true);
    await flush();
    expect(starts).toHaveLength(3);

    starts[0]!.deferred.resolve(makeSnapshot("preview-1", "w3-02-preview-1-1", first.previewSource, "ready"));
    starts[1]!.deferred.resolve(makeSnapshot("preview-1", "w3-02-preview-2-2", second.previewSource, "ready"));
    starts[2]!.deferred.resolve(makeSnapshot("preview-1", "w3-02-preview-3-3", third.previewSource, "ready"));
    await flush();

    expect(controller.getState().source?.previewSource).toEqual(third.previewSource);
    expect(controller.getState().phase).toBe("metadata_fallback");
    expect(snapshots.filter((snapshot) => snapshot?.state === "ready").every((snapshot) => snapshot?.source === third.previewSource)).toBe(true);
  });

  it("native_backend_fallback_updates_preview_experience_state", async () => {
    const { api, starts, resolveStart } = makePreviewApi();
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const current = source("file-native")!;
    const trigger = document.body.appendChild(document.createElement("button"));
    const native = makeNativeSnapshot(
      "preview-1",
      "w3-02-preview-1-1",
      current.previewSource,
      "native-version",
      "native-token"
    );
    const fallback = makeMetadataFallbackSnapshot(
      "preview-1",
      native.requestId,
      current.previewSource,
      "native-version"
    );

    controller.open(current, trigger);
    await flush();
    resolveStart(starts[0]!, native);
    await flush();
    expect(controller.getState().snapshot?.representation?.representation.family).toBe("native_opaque");

    const geometry = deferred<PreviewSnapshot | null>();
    vi.spyOn(workspace, "updateNativePreviewGeometry").mockReturnValue(geometry.promise);
    const update = controller.updateNativePreviewGeometry("preview-1", {
      host: "zen_floating",
      token: "native-token",
      sourceVersion: "native-version",
      bounds: { x: 0, y: 0, width: 320, height: 240 }
    });
    geometry.resolve(fallback);

    await expect(update).resolves.toEqual(fallback);
    expect(controller.getState().phase).toBe("metadata_fallback");
    expect(controller.getState().snapshot?.representation?.representation.family).toBe("metadata");
  });

  it("native_backend_fallback_unmounts_native_opaque_representation", async () => {
    const { api, starts, resolveStart } = makePreviewApi();
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const current = source("file-native-unmount")!;
    const trigger = document.body.appendChild(document.createElement("button"));
    const native = makeNativeSnapshot(
      "preview-1",
      "w3-02-preview-1-1",
      current.previewSource,
      "native-version",
      "native-token"
    );
    const fallback = makeMetadataFallbackSnapshot(
      "preview-1",
      native.requestId,
      current.previewSource,
      "native-version"
    );

    controller.open(current, trigger);
    await flush();
    resolveStart(starts[0]!, native);
    await flush();
    expect(controller.getState().snapshot?.representation?.representation.family).toBe("native_opaque");

    const geometry = deferred<PreviewSnapshot | null>();
    vi.spyOn(workspace, "updateNativePreviewGeometry").mockReturnValue(geometry.promise);
    const update = controller.updateNativePreviewGeometry("preview-1", {
      host: "zen_floating",
      token: "native-token",
      sourceVersion: "native-version",
      bounds: { x: 0, y: 0, width: 320, height: 240 }
    });
    geometry.resolve(fallback);
    await expect(update).resolves.toEqual(fallback);

    expect(controller.getState().phase).toBe("metadata_fallback");
    expect(controller.getState().snapshot?.representation?.representation.family).toBe("metadata");
    expect(controller.getState().snapshot?.representation?.representation.family).not.toBe("native_opaque");
  });

  it("stale_geometry_result_does_not_replace_current_preview_state", async () => {
    const { api, starts, resolveStart } = makePreviewApi();
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const current = source("file-native-null")!;
    const trigger = document.body.appendChild(document.createElement("button"));
    const native = makeNativeSnapshot(
      "preview-1",
      "w3-02-preview-1-1",
      current.previewSource,
      "native-version",
      "native-token"
    );

    controller.open(current, trigger);
    await flush();
    resolveStart(starts[0]!, native);
    await flush();
    const geometry = deferred<PreviewSnapshot | null>();
    vi.spyOn(workspace, "updateNativePreviewGeometry").mockReturnValue(geometry.promise);
    const update = controller.updateNativePreviewGeometry("preview-1", {
      host: "zen_floating",
      token: "native-token",
      sourceVersion: "native-version",
      bounds: { x: 0, y: 0, width: 320, height: 240 }
    });
    geometry.resolve(null);

    await expect(update).resolves.toBeNull();
    expect(controller.getState().snapshot).toEqual(native);
    expect(controller.getState().phase).toBe("content");
  });

  it("b_remains_visible_when_late_a_geometry_fails", async () => {
    const { api, starts, switches, resolveStart, resolveSwitch } = makePreviewApi({ deferSwitch: true });
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const first = source("file-native-a")!;
    const second = source("file-native-b")!;
    const trigger = document.body.appendChild(document.createElement("button"));
    const nativeA = makeNativeSnapshot(
      "preview-1",
      "w3-02-preview-a-1",
      first.previewSource,
      "native-version-a",
      "native-token-a"
    );
    const fallbackA = makeMetadataFallbackSnapshot(
      "preview-1",
      nativeA.requestId,
      first.previewSource,
      "native-version-a"
    );

    controller.open(first, trigger);
    await flush();
    resolveStart(starts[0]!, nativeA);
    await flush();
    const geometry = deferred<PreviewSnapshot | null>();
    vi.spyOn(workspace, "updateNativePreviewGeometry").mockReturnValue(geometry.promise);
    const update = controller.updateNativePreviewGeometry("preview-1", {
      host: "zen_floating",
      token: "native-token-a",
      sourceVersion: "native-version-a",
      bounds: { x: 0, y: 0, width: 320, height: 240 }
    });

    controller.open(second, trigger);
    await flush();
    expect(switches).toHaveLength(1);
    resolveSwitch(switches[0]!);
    await flush();
    expect(starts).toHaveLength(2);
    const nativeB = makeNativeSnapshot(
      "preview-1",
      starts[1]!.requestId,
      second.previewSource,
      "native-version-b",
      "native-token-b"
    );
    resolveStart(starts[1]!, nativeB);
    await flush();
    expect(controller.getState().snapshot?.representation?.representation).toEqual(
      nativeB.representation?.representation
    );

    geometry.resolve(fallbackA);
    await expect(update).resolves.toBeNull();
    expect(controller.getState().source?.previewSource).toEqual(second.previewSource);
    expect(controller.getState().snapshot?.requestId).toBe(nativeB.requestId);
    expect(controller.getState().snapshot?.representation?.representation).toEqual(
      nativeB.representation?.representation
    );
  });

  it("serializes B and coalesces C/D while preserving the latest backend truth", async () => {
    const { api, starts, switches, resolveStart, resolveSwitch, getBackendPreview, previewCancel, previewDispose } = makePreviewApi({ deferSwitch: true });
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const first = source("file-a")!;
    const second = source("file-b")!;
    const third = source("file-c")!;
    const fourth = source("file-d")!;
    const trigger = document.body.appendChild(document.createElement("button"));

    expect(controller.open(first, trigger)).toBe(true);
    await flush();
    expect(starts).toHaveLength(1);

    expect(controller.open(second, trigger)).toBe(true);
    await flush();
    expect(switches).toHaveLength(1);

    expect(controller.open(third, trigger)).toBe(true);
    await flush();
    expect(switches).toHaveLength(1);
    expect(controller.getState().source?.previewSource).toEqual(third.previewSource);
    expect(controller.getState().snapshot).toBeNull();

    expect(controller.open(fourth, trigger)).toBe(true);
    await flush();
    expect(switches).toHaveLength(1);
    expect(controller.getState().source?.previewSource).toEqual(fourth.previewSource);
    expect(previewCancel).not.toHaveBeenCalled();
    expect(previewDispose).not.toHaveBeenCalled();

    resolveSwitch(switches[0]!);
    await flush();
    expect(controller.getState().snapshot).toBeNull();
    expect(workspace.getState().previews["preview-1"]?.source).toEqual(first.previewSource);
    expect(getBackendPreview("preview-1")?.source).toEqual(second.previewSource);
    expect(switches).toHaveLength(2);
    expect(switches[1]?.source).toEqual(fourth.previewSource);
    expect(previewCancel).not.toHaveBeenCalled();
    expect(previewDispose).not.toHaveBeenCalled();

    resolveSwitch(switches[1]!);
    await flush();
    expect(starts).toHaveLength(2);
    expect(controller.getState().snapshot?.source).toEqual(fourth.previewSource);

    resolveStart(starts[1]!);
    await flush();
    resolveStart(starts[0]!);
    await flush();

    const cached = workspace.getState().previews[controller.getState().previewId!];
    expect(controller.getState().visible).toBe(true);
    expect(controller.getState().previewId).toBe("preview-1");
    expect(controller.getState().source?.previewSource).toEqual(fourth.previewSource);
    expect(controller.getState().snapshot?.source).toEqual(fourth.previewSource);
    expect(cached?.source).toEqual(fourth.previewSource);
    expect(getBackendPreview("preview-1")?.source).toEqual(fourth.previewSource);
    expect(previewCancel).not.toHaveBeenCalled();
    expect(previewDispose).not.toHaveBeenCalled();
  });

  it("retains the rapid B/C latest-wins scenario without concurrent switches", async () => {
    const { api, starts, switches, resolveStart, resolveSwitch, getBackendPreview, previewCancel, previewDispose } = makePreviewApi({ deferSwitch: true });
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const first = source("file-a")!;
    const second = source("file-b")!;
    const third = source("file-c")!;
    const trigger = document.body.appendChild(document.createElement("button"));

    controller.open(first, trigger);
    await flush();
    controller.open(second, trigger);
    await flush();
    controller.open(third, trigger);
    await flush();
    expect(switches).toHaveLength(1);
    expect(starts).toHaveLength(1);

    resolveSwitch(switches[0]!);
    await flush();
    expect(workspace.getState().previews["preview-1"]?.source).toEqual(first.previewSource);
    expect(getBackendPreview("preview-1")?.source).toEqual(second.previewSource);
    expect(switches).toHaveLength(2);
    expect(switches[1]?.source).toEqual(third.previewSource);

    resolveSwitch(switches[1]!);
    await flush();
    expect(starts).toHaveLength(2);
    resolveStart(starts[1]!);
    await flush();
    resolveStart(starts[0]!);
    await flush();

    expect(controller.getState().snapshot?.source).toEqual(third.previewSource);
    expect(workspace.getState().previews["preview-1"]?.source).toEqual(third.previewSource);
    expect(getBackendPreview("preview-1")?.source).toEqual(third.previewSource);
    expect(previewCancel).not.toHaveBeenCalled();
    expect(previewDispose).not.toHaveBeenCalled();
  });

  it("does not let a late A start overwrite the cache after switching to B", async () => {
    const { api, starts, resolveStart } = makePreviewApi();
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const first = source("file-a")!;
    const second = source("file-b")!;
    const trigger = document.body.appendChild(document.createElement("button"));

    controller.open(first, trigger);
    await flush();
    controller.open(second, trigger);
    await flush();
    expect(starts).toHaveLength(2);

    resolveStart(starts[1]!);
    await flush();
    resolveStart(starts[0]!);
    await flush();

    expect(controller.getState().snapshot?.source).toEqual(second.previewSource);
    expect(workspace.getState().previews["preview-1"]?.source).toEqual(second.previewSource);
    await expect(workspace.snapshotPreview("preview-1")).resolves.toEqual(expect.objectContaining({
      requestId: starts[1]!.requestId,
      source: second.previewSource
    }));
  });

  it("publishes an observed terminal warning before a pending previewStart rejects", async () => {
    const fixture = makePreviewApi({ deferSnapshots: true });
    const workspace = new FileWorkspaceController(fixture.api);
    const controller = new PreviewExperienceController(workspace);
    const current = source("terminal-pending")!;
    const trigger = document.body.appendChild(document.createElement("button"));

    expect(controller.open(current, trigger)).toBe(true);
    await flush();
    expect(fixture.starts).toHaveLength(1);
    expect(fixture.snapshots).toHaveLength(1);

    fixture.snapshots[0]!.deferred.resolve(
      makeTerminalMetadataSnapshot(
        "preview-1",
        fixture.starts[0]!.requestId,
        current.previewSource,
        "materialization_required"
      )
    );
    await flush();

    expect(controller.getState().phase).toBe("materialization_required");
    expect(controller.getState().phase).not.toBe("error");
    expect(controller.getState().phase).not.toBe("metadata_fallback");

    fixture.rejectStart(fixture.starts[0]!, "preview_materialization_required");
    await flush();
    expect(controller.getState().phase).toBe("materialization_required");
  });

  it.each([
    ["source_unavailable", "source_unavailable"],
    ["materialization_required", "materialization_required"],
    ["permission_denied", "permission_denied"],
    ["identity_changed", "identity_changed"],
    ["cancelled", "cancelled"]
  ] as const)("maps typed terminal warning %s without generic fallback", async (condition, expectedPhase) => {
    const fixture = makePreviewApi({ deferSnapshots: true });
    const workspace = new FileWorkspaceController(fixture.api);
    const controller = new PreviewExperienceController(workspace);
    const current = source(`terminal-${condition}`)!;
    const trigger = document.body.appendChild(document.createElement("button"));

    controller.open(current, trigger);
    await flush();
    fixture.snapshots[0]!.deferred.resolve(
      makeTerminalMetadataSnapshot("preview-1", fixture.starts[0]!.requestId, current.previewSource, condition)
    );
    await flush();

    expect(controller.getState().phase).toBe(expectedPhase);
    expect(controller.getState().phase).not.toBe("error");
    expect(controller.getState().phase).not.toBe("metadata_fallback");
  });

  it("does not let a late terminal A snapshot or rejection alter current B", async () => {
    const fixture = makePreviewApi({ deferSnapshots: true });
    const workspace = new FileWorkspaceController(fixture.api);
    const controller = new PreviewExperienceController(workspace);
    const first = source("terminal-a")!;
    const second = source("current-b")!;
    const trigger = document.body.appendChild(document.createElement("button"));

    controller.open(first, trigger);
    await flush();
    controller.open(second, trigger);
    await flush();
    expect(fixture.starts).toHaveLength(2);
    expect(fixture.snapshots).toHaveLength(2);

    const currentSnapshot = makeSnapshot("preview-1", fixture.starts[1]!.requestId, second.previewSource, "ready");
    fixture.snapshots[1]!.deferred.resolve(currentSnapshot);
    await flush();
    expect(controller.getState().source?.previewSource).toEqual(second.previewSource);
    expect(controller.getState().snapshot).toEqual(currentSnapshot);
    expect(controller.getState().phase).toBe("metadata_fallback");

    fixture.snapshots[0]!.deferred.resolve(
      makeTerminalMetadataSnapshot(
        "preview-1",
        fixture.starts[0]!.requestId,
        first.previewSource,
        "materialization_required"
      )
    );
    fixture.rejectStart(fixture.starts[0]!, "preview_materialization_required");
    await flush();

    expect(controller.getState().source?.previewSource).toEqual(second.previewSource);
    expect(controller.getState().snapshot).toEqual(currentSnapshot);
    expect(controller.getState().phase).toBe("metadata_fallback");
    expect(workspace.getState().previews["preview-1"]?.source).toEqual(second.previewSource);
  });

  it("publishes bounded progressive snapshots before start settles and ignores stale A observation", async () => {
    vi.useFakeTimers();
    const progressiveApi = makePreviewApi({ deferSnapshots: true });
    const workspace = new FileWorkspaceController(progressiveApi.api);
    const controller = new PreviewExperienceController(workspace);
    const first = source("folder-a")!;
    const second = source("folder-b")!;
    const trigger = document.body.appendChild(document.createElement("button"));
    const workspaceSessionBefore = workspace.getState().session;

    controller.open(first, trigger);
    await flush();
    expect(progressiveApi.starts).toHaveLength(1);
    expect(progressiveApi.snapshots).toHaveLength(1);
    progressiveApi.snapshots[0]!.deferred.resolve(
      makeProgressiveFolderSnapshot("preview-1", progressiveApi.starts[0]!.requestId, first.previewSource, 1)
    );
    await flush();
    expect(controller.getState().snapshot?.representation?.completeness).toBe("partial");
    expect(controller.getState().snapshot?.representation?.representation).toEqual({
      family: "folder_summary",
      encodedSummary: "folder-1"
    });

    vi.advanceTimersByTime(250);
    await flush();
    expect(progressiveApi.snapshots).toHaveLength(2);

    controller.open(second, trigger);
    await flush();
    expect(progressiveApi.starts).toHaveLength(2);
    expect(progressiveApi.snapshots).toHaveLength(3);

    progressiveApi.snapshots[1]!.deferred.resolve(
      makeProgressiveFolderSnapshot("preview-1", progressiveApi.starts[0]!.requestId, first.previewSource, 2)
    );
    await flush();
    expect(controller.getState().source?.previewSource).toEqual(second.previewSource);
    expect(controller.getState().snapshot?.representation?.representation).not.toEqual({
      family: "folder_summary",
      encodedSummary: "folder-2"
    });

    progressiveApi.snapshots[2]!.deferred.resolve(
      makeProgressiveFolderSnapshot("preview-1", progressiveApi.starts[1]!.requestId, second.previewSource, 1)
    );
    await flush();
    expect(controller.getState().snapshot?.source).toEqual(second.previewSource);
    expect(controller.getState().snapshot?.representation?.completeness).toBe("partial");

    progressiveApi.starts[1]!.deferred.resolve(
      makeProgressiveFolderSnapshot("preview-1", progressiveApi.starts[1]!.requestId, second.previewSource, 4, "complete")
    );
    await flush();
    expect(controller.getState().snapshot?.source).toEqual(second.previewSource);
    expect(controller.getState().snapshot?.representation?.completeness).toBe("complete");
    expect(workspace.getState().session).toEqual(workspaceSessionBefore);
    expect(controller.getState().source?.previewSource).toEqual(second.previewSource);

    const snapshotCallsAfterFinal = progressiveApi.snapshots.length;
    vi.advanceTimersByTime(4_000);
    await flush();
    expect(progressiveApi.snapshots).toHaveLength(snapshotCallsAfterFinal);
  });

  it("stops the pending snapshot observer on close and dispose", async () => {
    vi.useFakeTimers();
    const progressiveApi = makePreviewApi({ deferSnapshots: true });
    const workspace = new FileWorkspaceController(progressiveApi.api);
    const controller = new PreviewExperienceController(workspace);
    const current = source("folder-close")!;
    const trigger = document.body.appendChild(document.createElement("button"));

    controller.open(current, trigger);
    await flush();
    expect(progressiveApi.snapshots).toHaveLength(1);
    expect(controller.close("button")).toBe(true);
    progressiveApi.snapshots[0]!.deferred.resolve(
      makeProgressiveFolderSnapshot("preview-1", progressiveApi.starts[0]!.requestId, current.previewSource, 1)
    );
    await flush();
    vi.advanceTimersByTime(4_000);
    await flush();
    expect(progressiveApi.snapshots).toHaveLength(1);

    controller.open(current, trigger);
    await flush();
    expect(progressiveApi.snapshots).toHaveLength(2);
    await controller.dispose();
    progressiveApi.snapshots[1]!.deferred.resolve(
      makeProgressiveFolderSnapshot("preview-2", progressiveApi.starts[1]!.requestId, current.previewSource, 1)
    );
    await flush();
    vi.advanceTimersByTime(4_000);
    await flush();
    expect(progressiveApi.snapshots).toHaveLength(2);
  });

  it("guards input, IME and Alt+Space, and disposes one pending session once", async () => {
    const { api, starts, previewCancel, previewDispose } = makePreviewApi();
    const workspace = new FileWorkspaceController(api);
    const controller = new PreviewExperienceController(workspace);
    const current = source("file-a")!;
    const trigger = document.body.appendChild(document.createElement("button"));
    const input = document.body.appendChild(document.createElement("input"));
    const menu = document.body.appendChild(document.createElement("div"));
    menu.setAttribute("role", "menu");
    const guarded = (overrides: Partial<PreviewSpaceEvent>): PreviewSpaceEvent => ({
      altKey: false,
      isComposing: false,
      target: input,
      ...overrides
    });

    expect(controller.handleSpace(current, trigger, guarded({}))).toBe(false);
    expect(controller.handleSpace(current, trigger, guarded({ target: trigger, isComposing: true }))).toBe(false);
    expect(controller.handleSpace(current, trigger, guarded({ target: trigger, altKey: true }))).toBe(false);
    expect(controller.handleSpace(current, trigger, guarded({ target: trigger, repeat: true }))).toBe(false);
    expect(controller.handleSpace(current, trigger, guarded({ target: menu }))).toBe(false);
    expect(controller.handleSpace(current, trigger, guarded({ target: trigger, defaultPrevented: true }))).toBe(false);
    expect(controller.handleSpace(null, trigger, guarded({ target: trigger }))).toBe(false);

    expect(controller.handleSpace(current, trigger, guarded({ target: trigger }))).toBe(true);
    await flush();
    expect(starts).toHaveLength(1);
    expect(controller.handleSpace(current, trigger, guarded({ target: trigger }))).toBe(true);
    await flush();
    expect(previewCancel).toHaveBeenCalledTimes(1);
    expect(previewDispose).toHaveBeenCalledTimes(1);

    starts[0]!.deferred.resolve(makeSnapshot("preview-1", "w3-02-preview-1-1", current.previewSource, "ready"));
    await flush();
    expect(previewCancel).toHaveBeenCalledTimes(1);
    expect(previewDispose).toHaveBeenCalledTimes(1);
  });

  it("releases Preview disposal state across repeated open/close cycles", async () => {
    const fixture = makePreviewApi();
    const workspace = new FileWorkspaceController(fixture.api);
    const controller = new PreviewExperienceController(workspace);
    const trigger = document.body.appendChild(document.createElement("button"));

    for (let cycle = 0; cycle < 100; cycle += 1) {
      const current = source(`steady-${cycle}`)!;
      expect(controller.open(current, trigger)).toBe(true);
      await flush();
      expect(fixture.starts).toHaveLength(cycle + 1);

      fixture.resolveStart(fixture.starts[cycle]!);
      await flush();
      expect(controller.close("button")).toBe(true);
      await flush();
    }

    const internals = workspace as unknown as {
      ownedPreviewIds: Set<string>;
      previewDisposals: Map<string, Promise<boolean>>;
      previewPublications: Map<string, unknown>;
      previewsValue: Map<string, PreviewSnapshot>;
    };
    expect(fixture.previewCancel).toHaveBeenCalledTimes(100);
    expect(fixture.previewDispose).toHaveBeenCalledTimes(100);
    expect(internals.ownedPreviewIds.size).toBe(0);
    expect(internals.previewDisposals.size).toBe(0);
    expect(internals.previewPublications.size).toBe(0);
    expect(internals.previewsValue.size).toBe(0);
    expect(fixture.getBackendPreview("preview-1")).toBeUndefined();
  });

  it("splits workspace Space ownership from Floating-local close ownership", () => {
    const dialog = document.body.appendChild(document.createElement("section"));
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    const content = dialog.appendChild(document.createElement("div"));
    const close = vi.fn(() => true);
    const event = (target: EventTarget, overrides: Partial<FloatingPreviewSpaceEvent> = {}): FloatingPreviewSpaceEvent => ({
      key: " ",
      altKey: false,
      defaultPrevented: false,
      isComposing: false,
      repeat: false,
      target,
      preventDefault: vi.fn(),
      ...overrides
    });

    expect(isPreviewWorkspaceSpaceEligible(event(content))).toBe(false);
    expect(isFloatingPreviewCloseSpaceEligible(event(content))).toBe(true);

    const normal = event(content);
    expect(handleFloatingPreviewSpace(normal, close)).toBe(true);
    expect(close).toHaveBeenCalledOnce();
    expect(normal.preventDefault).toHaveBeenCalledOnce();

    const textbox = dialog.appendChild(document.createElement("div"));
    textbox.setAttribute("role", "textbox");
    const guardedTargets = [
      event(content, { repeat: true }),
      event(content, { isComposing: true }),
      event(content, { altKey: true }),
      event(dialog.appendChild(document.createElement("input"))),
      event(dialog.appendChild(document.createElement("textarea"))),
      event(textbox),
      event(content, { defaultPrevented: true })
    ];
    for (const guarded of guardedTargets) {
      expect(handleFloatingPreviewSpace(guarded, close)).toBe(false);
    }

    for (const [tag, role] of [["button", undefined], ["a", undefined], ["div", "button"], ["div", "menuitem"], ["div", "option"]] as const) {
      const target = dialog.appendChild(document.createElement(tag));
      if (tag === "a") target.setAttribute("href", "#");
      if (role !== undefined) target.setAttribute("role", role);
      expect(isFloatingPreviewCloseSpaceEligible(event(target))).toBe(false);
      expect(handleFloatingPreviewSpace(event(target), close)).toBe(false);
    }
    expect(close).toHaveBeenCalledOnce();
  });

  it("leaves Enter alone and consumes Space only when preview accepts it", async () => {
    const entry = adaptLibrarySummary(summary("file-a"));
    const interaction = createLibraryInteractionProjection({
      files: [summary("file-a")],
      totalCount: 1,
      collection: adaptLibraryCollection({ queryFingerprint: "query", snapshotRevision: 1 }),
      focusedId: "file-a",
      selection: null,
      selectionContainsFileId: () => false,
      hasMore: false,
      isLoading: false,
      loadNextPage: async () => undefined,
      setExplicitSelection: () => undefined,
      setFocusedId: () => undefined,
      toggleSelection: () => undefined,
      selectAllMatching: () => undefined,
      clearSelection: () => undefined
    });
    const onPreview = vi.fn(() => true);
    const container = document.body.appendChild(document.createElement("div"));
    const root: Root = createRoot(container);
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, value: 220 });
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, value: 220 });
    class ResizeObserverStub { observe() {} unobserve() {} disconnect() {} }
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);

    await act(async () => {
      root.render(createElement(SharedFileList, {
        interaction,
        language: "en",
        t,
        ariaLabel: "Files",
        onPreview
      }));
      await Promise.resolve();
    });
    const list = container.querySelector<HTMLElement>("[role='listbox']");
    expect(list).not.toBeNull();
    const enter = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    list!.dispatchEvent(enter);
    expect(onPreview).not.toHaveBeenCalled();
    expect(enter.defaultPrevented).toBe(false);
    const space = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true });
    list!.dispatchEvent(space);
    expect(onPreview).toHaveBeenCalledWith(entry, list, expect.anything());
    expect(space.defaultPrevented).toBe(true);
    root.unmount();
  });

  it("preserves exact terminal UX phases without exposing unknown backend errors", async () => {
    expect(previewPhaseForBackendError(new Error("preview_materialization_required")))
      .toBe("materialization_required");
    expect(previewPhaseForBackendError("preview_permission_denied")).toBe("permission_denied");
    expect(previewPhaseForBackendError("preview_source_unavailable")).toBe("source_unavailable");
    expect(previewPhaseForBackendError("preview_source_identity_changed")).toBe("identity_changed");
    expect(previewPhaseForBackendError("preview_cancelled")).toBe("cancelled");
    expect(previewPhaseForBackendError(new Error("raw provider failure"))).toBe("error");

    const fixture = makePreviewApi({ startError: "preview_materialization_required" });
    const workspace = new FileWorkspaceController(fixture.api);
    const controller = new PreviewExperienceController(workspace);
    const current = source("materialization");
    const trigger = document.body.appendChild(document.createElement("button"));

    expect(controller.open(current, trigger)).toBe(true);
    await flush();
    expect(controller.getState()).toEqual(expect.objectContaining({
      phase: "materialization_required",
      source: current,
      snapshot: null
    }));
    expect(fixture.starts).toHaveLength(0);
  });
});

afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = "";
  if (nativeClientWidth) Object.defineProperty(HTMLElement.prototype, "clientWidth", nativeClientWidth);
  else delete (HTMLElement.prototype as { clientWidth?: number }).clientWidth;
  if (nativeClientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", nativeClientHeight);
  else delete (HTMLElement.prototype as { clientHeight?: number }).clientHeight;
  if (nativeOffsetHeight) Object.defineProperty(HTMLElement.prototype, "offsetHeight", nativeOffsetHeight);
  else delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
  vi.unstubAllGlobals();
});
