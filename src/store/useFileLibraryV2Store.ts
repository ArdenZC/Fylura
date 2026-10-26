import { create } from "zustand";
import { tauriApi } from "../api/tauriApi";
import type {
  CreateLibrarySavedViewRequest,
  CreateUserTagRequest,
  DeleteLibrarySavedViewRequest,
  DeleteUserTagRequest,
  FileLibraryDetail,
  FileLibraryScopeV2,
  FileLibrarySelectionSummary,
  FileLibrarySortV2,
  FileLibrarySummary,
  FileQueryFiltersV2,
  FileQueryRequestV2,
  FileQueryResponseV2,
  FileQuerySpecV2,
  LibrarySavedView,
  LibraryScope,
  LibraryScopeHealth,
  LibrarySelectionV1,
  MutateFileUserTagsRequest,
  MutateFileUserTagsResult,
  UserTag,
  UpdateLibrarySavedViewRequest,
  UpdateUserTagRequest
} from "../types/domain";
import { readableError } from "../utils/viewHelpers";

export const FILE_LIBRARY_V2_PAGE_SIZE = 50;

export const emptyFileQueryFilters: FileQueryFiltersV2 = {
  fileTypes: [],
  purposes: [],
  lifecycles: [],
  risks: [],
  sizeMin: null,
  sizeMax: null,
  modifiedFrom: null,
  modifiedTo: null,
  createdFrom: null,
  createdTo: null,
  duplicate: "any",
  review: "any",
  tagsAllOf: [],
  tagsAnyOf: [],
  tagsNoneOf: []
};

export const defaultFileLibraryQuerySpec: FileQuerySpecV2 = {
  scope: { kind: "all_enabled_roots" },
  text: null,
  filters: emptyFileQueryFilters,
  sort: { kind: "modified", direction: "desc" }
};

export function cloneFileQuerySpec(spec: FileQuerySpecV2): FileQuerySpecV2 {
  return {
    ...spec,
    scope: spec.scope.kind === "roots"
      ? { kind: "roots", scanRootIds: [...spec.scope.scanRootIds] }
      : spec.scope.kind === "current_scan"
        ? { kind: "current_scan", scanSessionId: spec.scope.scanSessionId }
        : { kind: "all_enabled_roots" },
    filters: {
      ...spec.filters,
      fileTypes: [...spec.filters.fileTypes],
      purposes: [...spec.filters.purposes],
      lifecycles: [...spec.filters.lifecycles],
      risks: [...spec.filters.risks],
      tagsAllOf: [...spec.filters.tagsAllOf],
      tagsAnyOf: [...spec.filters.tagsAnyOf],
      tagsNoneOf: [...spec.filters.tagsNoneOf]
    },
    sort: { ...spec.sort }
  };
}

export function legacyScopeToFileLibraryScope(scope: LibraryScope, roots: Array<{ id: string; normalizedPath: string }> = []): FileLibraryScopeV2 {
  if (scope.kind === "all") return { kind: "all_enabled_roots" };
  if (scope.kind === "current_scan" && scope.scanSessionId) {
    return { kind: "current_scan", scanSessionId: scope.scanSessionId };
  }
  const requestedPaths = scope.roots.map(normalizePathForComparison);
  const rootIds = roots
    .filter((root) => requestedPaths.includes(normalizePathForComparison(root.normalizedPath)))
    .map((root) => root.id)
    .filter((id, index, ids) => ids.indexOf(id) === index)
    .sort();
  return { kind: "roots", scanRootIds: rootIds };
}

export async function resolveLegacyLibraryScope(scope: LibraryScope): Promise<FileLibraryScopeV2> {
  if (scope.kind === "all" || (scope.kind === "current_scan" && scope.scanSessionId)) {
    return legacyScopeToFileLibraryScope(scope);
  }
  const roots = await tauriApi.listScanRoots();
  return legacyScopeToFileLibraryScope(scope, roots);
}

export function normalizePathForComparison(value: string) {
  return value.trim().replaceAll("\\", "/").replace(/\/+$/, "").toLowerCase();
}

export type InspectorDetailLoadResult =
  | { status: "applied"; detail: FileLibraryDetail; requestEpoch: number }
  | { status: "superseded"; requestEpoch: number }
  | { status: "failed"; error: string; requestEpoch: number };


const selectionMembershipCache = new WeakMap<readonly string[], ReadonlySet<string>>();

function selectionMembership(ids: readonly string[]): ReadonlySet<string> {
  const cached = selectionMembershipCache.get(ids);
  if (cached) return cached;
  const next = new Set(ids);
  selectionMembershipCache.set(ids, next);
  return next;
}

let projectionSerial = 0;

/** Each pane owns a projection over the same backend Query V2 contracts. */
export function createFileLibraryProjection() {
  const projectionId = ++projectionSerial;
  interface QueryState {
    spec: FileQuerySpecV2;
    fingerprint: string | null;
    snapshotRevision: number | null;
    scopeHealth: LibraryScopeHealth | null;
    setSpec: (spec: FileQuerySpecV2) => void;
    applyResponse: (response: FileQueryResponseV2) => void;
    clearSnapshot: () => void;
  }

  const useFileLibraryQueryStore = create<QueryState>((set) => ({
    spec: cloneFileQuerySpec(defaultFileLibraryQuerySpec),
    fingerprint: null,
    snapshotRevision: null,
    scopeHealth: null,
    setSpec: (spec) => set({ spec: cloneFileQuerySpec(spec), fingerprint: null, snapshotRevision: null }),
    applyResponse: (response) => set({
      fingerprint: response.queryFingerprint,
      snapshotRevision: response.snapshotRevision,
      scopeHealth: response.scopeHealth
    }),
    clearSnapshot: () => set({ fingerprint: null, snapshotRevision: null, scopeHealth: null })
  }));

  interface ResultState {
    files: FileLibrarySummary[];
    totalCount: number | null;
    countState: "exact" | "deferred";
    countToken: string | null;
    isCountLoading: boolean;
    nextCursor: string | null;
    hasMore: boolean;
    resultState: string;
    isLoading: boolean;
    error: string | null;
    requestEpoch: number;
    activeQueryKey: string | null;
    loadFirstPage: (spec?: FileQuerySpecV2, invalidate?: boolean) => Promise<void>;
    loadNextPage: () => Promise<void>;
    refresh: () => Promise<void>;
    clear: () => void;
  }

  function nextRequestId(epoch: number) {
    return `library-v2-${projectionId}-${epoch}-${Date.now().toString(36)}`.slice(0, 128);
  }

  async function executeLibraryQuery(
    spec: FileQuerySpecV2,
    pageSize: number,
    cursor: string | null,
    epoch: number
  ): Promise<FileQueryResponseV2> {
    const request: FileQueryRequestV2 = {
      version: 2,
      requestId: nextRequestId(epoch),
      query: cloneFileQuerySpec(spec),
      pageSize,
      cursor
    };
    return tauriApi.queryFileLibraryV2(request);
  }

  const useFileLibraryResultStore = create<ResultState>((set, get) => ({
    files: [],
    totalCount: 0,
    countState: "exact",
    countToken: null,
    isCountLoading: false,
    nextCursor: null,
    hasMore: false,
    resultState: "empty",
    isLoading: false,
    error: null,
    requestEpoch: 0,
    activeQueryKey: null,
    loadFirstPage: async (spec, invalidate = false) => {
      const queryStore = useFileLibraryQueryStore.getState();
      const nextSpec = spec ?? queryStore.spec;
      const activeQueryKey = JSON.stringify(nextSpec);
      if (!invalidate && get().isLoading && get().activeQueryKey === activeQueryKey) return;
      const epoch = get().requestEpoch + 1;
      set({ isLoading: true, error: null, requestEpoch: epoch, activeQueryKey, files: [], nextCursor: null, hasMore: false, totalCount: 0, countState: "exact", countToken: null, isCountLoading: false });
      try {
        const response = await executeLibraryQuery(nextSpec, FILE_LIBRARY_V2_PAGE_SIZE, null, epoch);
        if (epoch !== get().requestEpoch) return;
        useFileLibraryQueryStore.getState().applyResponse(response);
        set({
          files: response.files,
          totalCount: response.totalCount,
          countState: response.countState,
          countToken: response.countToken,
          isCountLoading: response.countState === "deferred",
          nextCursor: response.nextCursor,
          hasMore: response.hasMore,
          resultState: response.resultState,
          isLoading: false,
          activeQueryKey: null,
          error: response.resultState === "snapshot_expired" ? "library_snapshot_expired" : null
        });
        if (response.countState === "deferred" && response.countToken) {
          void tauriApi.resolveFileLibraryExactCountV2({
            version: 2,
            requestId: nextRequestId(epoch),
            countToken: response.countToken
          }).then((count) => {
            if (epoch !== get().requestEpoch) return;
            const query = useFileLibraryQueryStore.getState();
            if (count.queryFingerprint !== query.fingerprint || count.snapshotRevision !== query.snapshotRevision) return;
            set({ totalCount: count.totalCount, countState: "exact", countToken: null, isCountLoading: false });
          }).catch((error) => {
            if (epoch !== get().requestEpoch) return;
            const message = readableError(error);
            if (message.includes("library_snapshot_expired")) {
              set({ resultState: "snapshot_expired", error: "library_snapshot_expired", isCountLoading: false });
              if (useFileLibrarySelectionStore.getState().selection?.kind === "all_matching") {
                useFileLibrarySelectionStore.getState().clear();
              }
            } else {
              set({ error: message, isCountLoading: false });
            }
          });
        }
      } catch (error) {
        if (epoch !== get().requestEpoch) return;
        const message = readableError(error);
        set({ isLoading: false, activeQueryKey: null, error: message, resultState: message.includes("library_snapshot_expired") ? "snapshot_expired" : "failed" });
        if (message.includes("library_snapshot_expired") && useFileLibrarySelectionStore.getState().selection?.kind === "all_matching") {
          useFileLibrarySelectionStore.getState().clear();
        }
      }
    },
    loadNextPage: async () => {
      const cursor = get().nextCursor;
      if (get().isLoading || !get().hasMore || !cursor) return;
      const epoch = get().requestEpoch;
      set({ isLoading: true, activeQueryKey: null, error: null });
      try {
        const response = await executeLibraryQuery(
          useFileLibraryQueryStore.getState().spec,
          FILE_LIBRARY_V2_PAGE_SIZE,
          cursor,
          epoch
        );
        if (epoch !== get().requestEpoch) return;
        useFileLibraryQueryStore.getState().applyResponse(response);
        if (response.resultState === "snapshot_expired") {
          set({ isLoading: false, activeQueryKey: null, error: "library_snapshot_expired", resultState: response.resultState });
          if (useFileLibrarySelectionStore.getState().selection?.kind === "all_matching") {
            useFileLibrarySelectionStore.getState().clear();
          }
          return;
        }
        set((state) => ({
          files: [...state.files, ...response.files],
          totalCount: response.totalCount ?? state.totalCount,
          countState: state.countState === "exact" && state.totalCount !== null ? "exact" : response.countState,
          countToken: state.countState === "exact" ? null : response.countToken,
          nextCursor: response.nextCursor,
          hasMore: response.hasMore,
          resultState: response.resultState,
          isLoading: false,
          activeQueryKey: null,
          error: null
        }));
      } catch (error) {
        if (epoch !== get().requestEpoch) return;
        const message = readableError(error);
        set({
          isLoading: false,
          activeQueryKey: null,
          error: message,
          resultState: message.includes("library_snapshot_expired") ? "snapshot_expired" : "failed"
        });
        if (message.includes("library_snapshot_expired") && useFileLibrarySelectionStore.getState().selection?.kind === "all_matching") {
          useFileLibrarySelectionStore.getState().clear();
        }
      }
    },
    // A filesystem publication invalidates even an identical in-flight query.
    refresh: async () => get().loadFirstPage(undefined, true),
    clear: () => {
      const requestEpoch = get().requestEpoch + 1;
      set({
        files: [],
        totalCount: 0,
        countState: "exact",
        countToken: null,
        isCountLoading: false,
        nextCursor: null,
        hasMore: false,
        resultState: "empty",
        isLoading: false,
        error: null,
        requestEpoch,
        activeQueryKey: null
      });
      useFileLibraryQueryStore.getState().clearSnapshot();
    }
  }));

  interface SelectionState {
    selection: LibrarySelectionV1 | null;
    focusedId: string;
    anchorIndex: number;
    setExplicit: (fileIds: string[], focusedId?: string, anchorIndex?: number) => void;
    setFocused: (focusedId: string, anchorIndex?: number) => void;
    toggle: (fileId: string, loadedIds: string[], range?: boolean) => void;
    selectAllMatching: () => void;
    clear: () => void;
    isSelected: (fileId: string) => boolean;
  }

  const useFileLibrarySelectionStore = create<SelectionState>((set, get) => ({
    selection: null,
    focusedId: "",
    anchorIndex: -1,
    setExplicit: (fileIds, focusedId = fileIds[0] ?? "", anchorIndex = fileIds.length ? 0 : -1) => {
      const ids = [...new Set(fileIds)].slice(0, 100_000);
      set({ selection: ids.length ? { kind: "explicit", fileIds: ids } : null, focusedId, anchorIndex });
    },
    setFocused: (focusedId, anchorIndex = get().anchorIndex) => set({ focusedId, anchorIndex }),
    toggle: (fileId, loadedIds, range = false) => {
      const current = get().selection;
      if (current?.kind === "all_matching") {
        const excluded = new Set(current.excludedFileIds);
        if (excluded.has(fileId)) excluded.delete(fileId);
        else excluded.add(fileId);
        set({ selection: { ...current, excludedFileIds: [...excluded].sort() }, focusedId: fileId });
        return;
      }
      const currentIds = current?.kind === "explicit" ? current.fileIds : [];
      const index = loadedIds.indexOf(fileId);
      let nextIds = currentIds;
      if (range && index >= 0 && get().anchorIndex >= 0) {
        const start = Math.min(get().anchorIndex, index);
        const end = Math.max(get().anchorIndex, index);
        nextIds = [...new Set([...currentIds, ...loadedIds.slice(start, end + 1)])];
      } else if (currentIds.includes(fileId)) {
        nextIds = currentIds.filter((id) => id !== fileId);
      } else {
        nextIds = [...currentIds, fileId];
      }
      set({
        selection: nextIds.length ? { kind: "explicit", fileIds: nextIds.slice(0, 100_000) } : null,
        focusedId: fileId,
        anchorIndex: index
      });
    },
    selectAllMatching: () => {
      const query = useFileLibraryQueryStore.getState();
      if (!query.fingerprint || query.snapshotRevision === null) return;
      set({
        selection: {
          kind: "all_matching",
          query: cloneFileQuerySpec(query.spec),
          queryFingerprint: query.fingerprint,
          snapshotRevision: query.snapshotRevision,
          excludedFileIds: []
        }
      });
    },
    clear: () => set({ selection: null, focusedId: "", anchorIndex: -1 }),
    isSelected: (fileId) => {
      const selection = get().selection;
      if (!selection) return false;
      return selection.kind === "explicit"
        ? selectionMembership(selection.fileIds).has(fileId)
        : !selectionMembership(selection.excludedFileIds).has(fileId);
    }
  }));


  type DetailRequestEntry = {
    fileId: string;
    inspectorEpoch: number;
    promise: Promise<InspectorDetailLoadResult>;
  };

  interface InspectorState {
    detail: FileLibraryDetail | null;
    selectionSummary: FileLibrarySelectionSummary | null;
    selectedId: string | null;
    requestEpoch: number;
    isLoading: boolean;
    error: string | null;
    loadDetail: (fileId: string | null, invalidate?: boolean) => Promise<InspectorDetailLoadResult>;
    commitDetailIfCurrent: (fileId: string, detail: FileLibraryDetail, expectedEpoch: number) => boolean;
    loadSelectionSummary: (selection: LibrarySelectionV1 | null) => Promise<void>;
    clear: () => void;
  }

  const useFileLibraryInspectorStore = create<InspectorState>((set, get) => {
    let inFlightDetailRequest: DetailRequestEntry | null = null;

    return {
    detail: null,
    selectionSummary: null,
    selectedId: null,
    requestEpoch: 0,
    isLoading: false,
    error: null,
    loadDetail: async (fileId, invalidate = false) => {
      if (!fileId) {
        inFlightDetailRequest = null;
        const epoch = get().requestEpoch + 1;
        set({ selectedId: null, detail: null, selectionSummary: null, isLoading: false, error: null, requestEpoch: epoch });
        return { status: "superseded", requestEpoch: epoch };
      }
      const current = get();
      const existing = inFlightDetailRequest;
      if (!invalidate && existing
        && existing.fileId === fileId
        && existing.inspectorEpoch === current.requestEpoch
        && current.selectedId === fileId) return existing.promise;

      const epoch = current.requestEpoch + 1;
      set({ selectedId: fileId, detail: null, selectionSummary: null, isLoading: true, error: null, requestEpoch: epoch });
      const request: Promise<InspectorDetailLoadResult> = (async () => {
        try {
          const detail = await tauriApi.getFileLibraryDetail(fileId);
          if (epoch !== get().requestEpoch || get().selectedId !== fileId) return { status: "superseded", requestEpoch: epoch };
          set({ detail, isLoading: false, error: null });
          return { status: "applied", detail, requestEpoch: epoch };
        } catch (error) {
          if (epoch !== get().requestEpoch || get().selectedId !== fileId) return { status: "superseded", requestEpoch: epoch };
          const message = readableError(error);
          set({ isLoading: false, error: message });
          return { status: "failed", error: message, requestEpoch: epoch };
        } finally {
          if (inFlightDetailRequest?.fileId === fileId && inFlightDetailRequest.inspectorEpoch === epoch) inFlightDetailRequest = null;
        }
      })();
      inFlightDetailRequest = { fileId, inspectorEpoch: epoch, promise: request };
      return request;
    },
    commitDetailIfCurrent: (fileId, detail, expectedEpoch) => {
      const state = get();
      if (state.requestEpoch !== expectedEpoch || state.selectedId !== fileId) return false;
      set({ detail, isLoading: false, error: null });
      return true;
    },
    loadSelectionSummary: async (selection) => {
      inFlightDetailRequest = null;
      const epoch = get().requestEpoch + 1;
      set({ selectedId: null, detail: null, selectionSummary: null, isLoading: Boolean(selection), error: null, requestEpoch: epoch });
      if (!selection) return;
      try {
        const summary = await tauriApi.getFileLibrarySelectionSummary(selection);
        if (epoch !== get().requestEpoch) return;
        set({ selectionSummary: summary, isLoading: false });
      } catch (error) {
        if (epoch !== get().requestEpoch) return;
        set({ isLoading: false, error: readableError(error) });
      }
    },
    clear: () => {
      inFlightDetailRequest = null;
      set((state) => ({ detail: null, selectionSummary: null, selectedId: null, isLoading: false, error: null, requestEpoch: state.requestEpoch + 1 }));
    }
    };
  });

  /** Refresh the managed Query V2 projection after a durable filesystem publication. */
  async function refreshManagedFileLibraryProjection(): Promise<void> {
    const inspector = useFileLibraryInspectorStore.getState();
    const selection = useFileLibrarySelectionStore.getState().selection;
    const detail = inspector.selectedId !== null
      ? inspector.loadDetail(inspector.selectedId, true)
      : selection ? inspector.loadSelectionSummary(selection) : Promise.resolve();
    await Promise.all([useFileLibraryResultStore.getState().refresh(), detail]);
  }

  interface TagState {
    tags: UserTag[];
    isLoading: boolean;
    error: string | null;
    load: () => Promise<void>;
    create: (request: CreateUserTagRequest) => Promise<UserTag>;
    update: (request: UpdateUserTagRequest) => Promise<UserTag>;
    remove: (request: DeleteUserTagRequest) => Promise<boolean>;
    mutate: (request: MutateFileUserTagsRequest) => Promise<MutateFileUserTagsResult>;
  }

  const useFileLibraryTagStore = create<TagState>((set, get) => ({
    tags: [],
    isLoading: false,
    error: null,
    load: async () => {
      set({ isLoading: true, error: null });
      try {
        set({ tags: await tauriApi.listUserTags(), isLoading: false });
      } catch (error) {
        set({ isLoading: false, error: readableError(error) });
      }
    },
    create: async (request) => {
      const tag = await tauriApi.createUserTag(request);
      set({ tags: [...get().tags, tag].sort(compareTags) });
      return tag;
    },
    update: async (request) => {
      const tag = await tauriApi.updateUserTag(request);
      set({ tags: get().tags.map((item) => item.id === tag.id ? tag : item).sort(compareTags) });
      return tag;
    },
    remove: async (request) => {
      const removed = await tauriApi.deleteUserTag(request);
      if (removed) set({ tags: get().tags.filter((tag) => tag.id !== request.id) });
      return removed;
    },
    mutate: async (request) => {
      const result = await tauriApi.mutateFileUserTags(request);
      await get().load();
      return result;
    }
  }));

  interface SavedViewState {
    views: LibrarySavedView[];
    activeViewId: string | null;
    isLoading: boolean;
    error: string | null;
    load: () => Promise<void>;
    create: (request: CreateLibrarySavedViewRequest) => Promise<LibrarySavedView>;
    update: (request: UpdateLibrarySavedViewRequest) => Promise<LibrarySavedView>;
    remove: (request: DeleteLibrarySavedViewRequest) => Promise<boolean>;
    setActiveViewId: (id: string | null) => void;
  }

  const useFileLibrarySavedViewStore = create<SavedViewState>((set, get) => ({
    views: [],
    activeViewId: null,
    isLoading: false,
    error: null,
    load: async () => {
      set({ isLoading: true, error: null });
      try {
        set({ views: await tauriApi.listLibrarySavedViews(), isLoading: false });
      } catch (error) {
        set({ isLoading: false, error: readableError(error) });
      }
    },
    create: async (request) => {
      const view = await tauriApi.createLibrarySavedView(request);
      set({ views: [...get().views, view].sort(compareSavedViews), activeViewId: view.id });
      return view;
    },
    update: async (request) => {
      const view = await tauriApi.updateLibrarySavedView(request);
      set({ views: get().views.map((item) => item.id === view.id ? view : item).sort(compareSavedViews) });
      return view;
    },
    remove: async (request) => {
      const removed = await tauriApi.deleteLibrarySavedView(request);
      if (removed) set({ views: get().views.filter((view) => view.id !== request.id), activeViewId: get().activeViewId === request.id ? null : get().activeViewId });
      return removed;
    },
    setActiveViewId: (activeViewId) => set({ activeViewId })
  }));


  return { useFileLibraryQueryStore, useFileLibraryResultStore, useFileLibrarySelectionStore, useFileLibraryInspectorStore, useFileLibraryTagStore, useFileLibrarySavedViewStore, refreshManagedFileLibraryProjection };
}

export const defaultFileLibraryProjection = createFileLibraryProjection();
export const { useFileLibraryQueryStore, useFileLibrarySelectionStore, useFileLibraryInspectorStore, useFileLibraryTagStore, useFileLibrarySavedViewStore, refreshManagedFileLibraryProjection } = defaultFileLibraryProjection;
const defaultResultStore = defaultFileLibraryProjection.useFileLibraryResultStore;
export { defaultResultStore as useFileLibraryResultStore };

function compareTags(left: UserTag, right: UserTag) {
  return left.displayName.localeCompare(right.displayName) || left.id.localeCompare(right.id);
}

function compareSavedViews(left: LibrarySavedView, right: LibrarySavedView) {
  return left.position - right.position || right.updatedAt - left.updatedAt || left.id.localeCompare(right.id);
}

export function explicitSelectionIds(selection: LibrarySelectionV1 | null) {
  return selection?.kind === "explicit" ? selection.fileIds : [];
}

export function explicitSingleSelectionId(selection: LibrarySelectionV1 | null) {
  return selection?.kind === "explicit" && selection.fileIds.length === 1
    ? selection.fileIds[0]
    : null;
}

export function selectedLoadedIds(files: FileLibrarySummary[], selection: LibrarySelectionV1 | null) {
  if (!selection) return new Set<string>();
  const membership = selection.kind === "explicit"
    ? selectionMembership(selection.fileIds)
    : selectionMembership(selection.excludedFileIds);
  const selected = new Set<string>();
  for (const file of files) {
    if (selection.kind === "explicit" ? membership.has(file.id) : !membership.has(file.id)) selected.add(file.id);
  }
  return selected;
}

export function selectionContainsFileId(selection: LibrarySelectionV1 | null, fileId: string) {
  if (!selection) return false;
  return selection.kind === "explicit"
    ? selectionMembership(selection.fileIds).has(fileId)
    : !selectionMembership(selection.excludedFileIds).has(fileId);
}

export function buildLegacySearchSpec(scope: FileLibraryScopeV2, text: string): FileQuerySpecV2 {
  return {
    scope,
    text: text.trim() || null,
    filters: { ...emptyFileQueryFilters },
    sort: { kind: text.trim() ? "relevance" : "modified", direction: "desc" }
  };
}

export function sortForLibrary(kind: FileLibrarySortV2["kind"], direction: FileLibrarySortV2["direction"]): FileLibrarySortV2 {
  return { kind, direction };
}
