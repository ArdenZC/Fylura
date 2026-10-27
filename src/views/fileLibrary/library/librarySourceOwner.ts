import { usePaneProjection, usePane } from "../workbench/PaneContext";
import { useCallback, useMemo } from "react";
import { useFileLibraryStore } from "../../../store/useFileLibraryStore";
import { useScanManagerStore } from "../../../store/useScanManagerStore";
import {
  selectedLoadedIds,
  useFileLibraryInspectorStore,
  useFileLibraryQueryStore,
  useFileLibraryResultStore,
  useFileLibrarySavedViewStore,
  useFileLibrarySelectionStore,
  useFileLibraryTagStore,
  type InspectorDetailLoadResult
} from "../../../store/useFileLibraryV2Store";
import { useOperationQueueStore } from "../../../store/useOperationQueueStore";
import type {
  FileLibraryDetail,
  FileLibraryScopeV2,
  FileLibrarySummary,
  FileQuerySpecV2,
  LibrarySavedView,
  LibraryScope,
  LibraryScopeHealth,
  LibrarySelectionV1,
  UserTag
} from "../../../types/domain";
import type {
  LibraryPresentationCollectionContext,
  LibraryPresentationEntry
} from "../presentation/contracts";
import { adaptLibraryCollection, adaptLibrarySummary } from "../presentation/adapters";
import { useVaultQueryController } from "../../vault/controllers/useVaultQueryController";

/**
 * The File Library source owner composes existing managed authorities and
 * exposes one source-specific projection for the W2 workspace. It owns no
 * query, selection, cache or persistence state of its own.
 */
export interface LibrarySourceOwner {
  readonly source: "library";
  readonly scope: LibraryScope;
  readonly stats: ReturnType<typeof useFileLibraryStore.getState>["stats"];
  readonly querySpec: FileQuerySpecV2;
  readonly queryFingerprint: string | null;
  readonly snapshotRevision: number | null;
  readonly collection: LibraryPresentationCollectionContext | null;
  readonly files: FileLibrarySummary[];
  readonly totalCount: number | null;
  readonly countState: "exact" | "deferred";
  readonly countToken: string | null;
  readonly isCountLoading: boolean;
  readonly hasMore: boolean;
  readonly isLoading: boolean;
  readonly resultState: string;
  readonly error: string | null;
  readonly scopeHealth: LibraryScopeHealth | null;
  readonly isEmptyCurrentScanScope: boolean;
  readonly selectedIds: ReadonlySet<string>;
  readonly selectedIdList: string[];
  readonly selectedFiles: FileLibrarySummary[];
  readonly selection: LibrarySelectionV1 | null;
  readonly ownsSingleFileSelection: (fileId: string) => boolean;
  readonly focusedId: string;
  readonly detail: FileLibraryDetail | null;
  readonly selectionSummary: ReturnType<typeof useFileLibraryInspectorStore.getState>["selectionSummary"];
  readonly isInspectorLoading: boolean;
  readonly inspectorError: string | null;
  readonly tags: UserTag[];
  readonly savedViews: LibrarySavedView[];
  readonly activeViewId: string | null;
  readonly presentationEntryAt: (index: number) => LibraryPresentationEntry | undefined;
  readonly setScope: (scope: LibraryScope) => void;
  /** Direct Query V2 scope binding for backend-issued managed-root refs. */
  readonly setQueryScope: (scope: FileLibraryScopeV2) => void;
  readonly chooseFolders: () => Promise<unknown>;
  readonly loadNextPage: () => Promise<void>;
  readonly refreshResults: () => Promise<void>;
  readonly loadDetail: (fileId: string | null) => Promise<InspectorDetailLoadResult>;
  readonly commitDetailIfCurrent: (fileId: string, detail: FileLibraryDetail, expectedEpoch: number) => boolean;
  readonly loadSelectionSummary: (selection: LibrarySelectionV1 | null) => Promise<void>;
  readonly clearInspector: () => void;
  readonly clearSelection: () => void;
  readonly setExplicitSelection: (fileIds: string[], focusedId?: string, anchorIndex?: number) => void;
  readonly setFocusedId: (focusedId: string, anchorIndex?: number) => void;
  /** Moves only the source-owned loaded focus, loading one normal next page at the edge. */
  readonly moveFocus: (direction: "previous" | "next") => Promise<boolean>;
  readonly toggleSelection: (fileId: string, loadedIds: string[], range?: boolean) => void;
  readonly selectAllMatching: () => void;
  readonly selectionContainsFileId: (fileId: string) => boolean;
  readonly loadTags: () => Promise<void>;
  readonly mutateTags: ReturnType<typeof useFileLibraryTagStore.getState>["mutate"];
  readonly loadSavedViews: () => Promise<void>;
  readonly setActiveViewId: (id: string | null) => void;
  readonly applySavedView: ReturnType<typeof useVaultQueryController>["applySavedView"];
  readonly librarySearch: string;
  readonly debouncedSearchQuery: string;
  readonly scopeReady: boolean;
  readonly querySpecSignature: string;
  readonly updateFilters: ReturnType<typeof useVaultQueryController>["updateFilters"];
  readonly clearFilters: ReturnType<typeof useVaultQueryController>["clearFilters"];
  readonly setSort: ReturnType<typeof useVaultQueryController>["setSort"];
  readonly handleLibrarySearchChange: ReturnType<typeof useVaultQueryController>["handleLibrarySearchChange"];
  readonly clearExecutionIntent: ReturnType<typeof useOperationQueueStore.getState>["clearExecutionIntent"];
  readonly refreshPreviewsForFiles: ReturnType<typeof useOperationQueueStore.getState>["refreshPreviewsForFiles"];
  readonly refreshPreviewsForSelection: ReturnType<typeof useOperationQueueStore.getState>["refreshPreviewsForSelection"];
  readonly setPreviewResult: ReturnType<typeof useOperationQueueStore.getState>["setPreviewResult"];
}

export function useLibrarySourceOwner({ onError }: { onError: (error: unknown) => void }): LibrarySourceOwner {
  const { useFileLibraryQueryStore, useFileLibraryResultStore, useFileLibrarySelectionStore, useFileLibraryInspectorStore, useFileLibraryTagStore, useFileLibrarySavedViewStore } = usePaneProjection();
  const pane = usePane();
  const globalScope = useFileLibraryStore((state) => state.scope);
  const scope = pane?.scope ?? globalScope;
  const stats = useFileLibraryStore((state) => state.stats);
  const globalSetScope = useFileLibraryStore((state) => state.setScope);
  const setScope = pane?.setScope ?? globalSetScope;
  const chooseFolders = useScanManagerStore((state) => state.handleChooseFolders);

  const querySpec = useFileLibraryQueryStore((state) => state.spec);
  const setQuerySpec = useFileLibraryQueryStore((state) => state.setSpec);
  const queryFingerprint = useFileLibraryQueryStore((state) => state.fingerprint);
  const snapshotRevision = useFileLibraryQueryStore((state) => state.snapshotRevision);
  const scopeHealth = useFileLibraryQueryStore((state) => state.scopeHealth);

  const files = useFileLibraryResultStore((state) => state.files);
  const totalCount = useFileLibraryResultStore((state) => state.totalCount);
  const countState = useFileLibraryResultStore((state) => state.countState);
  const countToken = useFileLibraryResultStore((state) => state.countToken);
  const isCountLoading = useFileLibraryResultStore((state) => state.isCountLoading);
  const hasMore = useFileLibraryResultStore((state) => state.hasMore);
  const isLoading = useFileLibraryResultStore((state) => state.isLoading);
  const resultState = useFileLibraryResultStore((state) => state.resultState);
  const error = useFileLibraryResultStore((state) => state.error);
  const loadFirstPage = useFileLibraryResultStore((state) => state.loadFirstPage);
  const loadNextPage = useFileLibraryResultStore((state) => state.loadNextPage);
  const refreshResults = useFileLibraryResultStore((state) => state.refresh);
  const clearResults = useFileLibraryResultStore((state) => state.clear);

  const selection = useFileLibrarySelectionStore((state) => state.selection);
  const focusedId = useFileLibrarySelectionStore((state) => state.focusedId);
  const setExplicitSelection = useFileLibrarySelectionStore((state) => state.setExplicit);
  const setFocusedId = useFileLibrarySelectionStore((state) => state.setFocused);
  const toggleSelection = useFileLibrarySelectionStore((state) => state.toggle);
  const selectAllMatching = useFileLibrarySelectionStore((state) => state.selectAllMatching);
  const clearSelection = useFileLibrarySelectionStore((state) => state.clear);
  const selectionContainsFileId = useCallback(
    (fileId: string) => useFileLibrarySelectionStore.getState().isSelected(fileId),
    []
  );
  const ownsSingleFileSelection = useCallback((fileId: string) => {
    const current = useFileLibrarySelectionStore.getState().selection;
    return current?.kind === "explicit" && current.fileIds.length === 1 && current.fileIds[0] === fileId;
  }, []);

  const detail = useFileLibraryInspectorStore((state) => state.detail);
  const selectionSummary = useFileLibraryInspectorStore((state) => state.selectionSummary);
  const isInspectorLoading = useFileLibraryInspectorStore((state) => state.isLoading);
  const inspectorError = useFileLibraryInspectorStore((state) => state.error);
  const loadDetail = useFileLibraryInspectorStore((state) => state.loadDetail);
  const commitDetailIfCurrent = useFileLibraryInspectorStore((state) => state.commitDetailIfCurrent);
  const loadSelectionSummary = useFileLibraryInspectorStore((state) => state.loadSelectionSummary);
  const clearInspector = useFileLibraryInspectorStore((state) => state.clear);

  const tags = useFileLibraryTagStore((state) => state.tags);
  const loadTags = useFileLibraryTagStore((state) => state.load);
  const mutateTags = useFileLibraryTagStore((state) => state.mutate);
  const savedViews = useFileLibrarySavedViewStore((state) => state.views);
  const activeViewId = useFileLibrarySavedViewStore((state) => state.activeViewId);
  const loadSavedViews = useFileLibrarySavedViewStore((state) => state.load);
  const setActiveViewId = useFileLibrarySavedViewStore((state) => state.setActiveViewId);

  const clearExecutionIntent = useOperationQueueStore((state) => state.clearExecutionIntent);
  const refreshPreviewsForFiles = useOperationQueueStore((state) => state.refreshPreviewsForFiles);
  const refreshPreviewsForSelection = useOperationQueueStore((state) => state.refreshPreviewsForSelection);
  const setPreviewResult = useOperationQueueStore((state) => state.setPreviewResult);

  const setQueryScope = useCallback((nextScope: FileLibraryScopeV2) => {
    const current = useFileLibraryQueryStore.getState().spec;
    setQuerySpec({ ...current, scope: nextScope });
  }, [setQuerySpec]);

  const queryController = useVaultQueryController({
    legacyScope: scope,
    querySpec,
    setQuerySpec,
    loadFirstPage,
    clearResults,
    onError,
    savedViews,
    activeViewId,
    setActiveViewId,
    clearSelection,
    clearSelectionOnMount: false
  });

  const selectedIds = useMemo(() => selectedLoadedIds(files, selection), [files, selection]);
  const selectedIdList = useMemo(() => [...selectedIds], [selectedIds]);
  const selectedFiles = useMemo(() => files.filter((file) => selectedIds.has(file.id)), [files, selectedIds]);
  const collection = useMemo<LibraryPresentationCollectionContext | null>(() => {
    if (queryFingerprint === null || snapshotRevision === null) return null;
    return adaptLibraryCollection({ queryFingerprint, snapshotRevision }, querySpec);
  }, [queryFingerprint, querySpec, snapshotRevision]);

  // The list asks for entries by virtual row index. This preserves W2-02's
  // window-only adapter shape even when the logical Query V2 collection is
  // large, and never touches LibrarySelectionV1's all_matching IDs.
  const presentationEntryAt = useCallback((index: number) => libraryPresentationEntryAt(files, index), [files]);

  const moveFocus = useCallback(async (direction: "previous" | "next") => {
    const resultBefore = useFileLibraryResultStore.getState();
    const queryBefore = useFileLibraryQueryStore.getState();
    const currentIndex = resultBefore.files.findIndex((file) => file.id === useFileLibrarySelectionStore.getState().focusedId);
    if (currentIndex < 0) return false;

    if (direction === "previous" && currentIndex > 0) {
      const previous = resultBefore.files[currentIndex - 1];
      if (previous === undefined) return false;
      useFileLibrarySelectionStore.getState().setFocused(previous.id, currentIndex - 1);
      return true;
    }

    if (direction === "next" && currentIndex + 1 < resultBefore.files.length) {
      const next = resultBefore.files[currentIndex + 1];
      if (next === undefined) return false;
      useFileLibrarySelectionStore.getState().setFocused(next.id, currentIndex + 1);
      return true;
    }

    if (direction !== "next"
      || !resultBefore.hasMore
      || resultBefore.isLoading) return false;

    const expectedRequestEpoch = resultBefore.requestEpoch;
    const expectedFingerprint = queryBefore.fingerprint;
    const expectedSnapshotRevision = queryBefore.snapshotRevision;
    const expectedFocusedId = useFileLibrarySelectionStore.getState().focusedId;
    await loadNextPage();

    const resultAfter = useFileLibraryResultStore.getState();
    const queryAfter = useFileLibraryQueryStore.getState();
    if (resultAfter.requestEpoch !== expectedRequestEpoch
      || queryAfter.fingerprint !== expectedFingerprint
      || queryAfter.snapshotRevision !== expectedSnapshotRevision
      || useFileLibrarySelectionStore.getState().focusedId !== expectedFocusedId) {
      return false;
    }
    const refreshedIndex = resultAfter.files.findIndex((file) => file.id === expectedFocusedId);
    const next = refreshedIndex < 0 ? undefined : resultAfter.files[refreshedIndex + 1];
    if (next === undefined) return false;
    useFileLibrarySelectionStore.getState().setFocused(next.id, refreshedIndex + 1);
    return true;
  }, [loadNextPage]);

  return {
    source: "library",
    scope,
    stats,
    querySpec,
    queryFingerprint,
    snapshotRevision,
    collection,
    files,
    totalCount,
    countState,
    countToken,
    isCountLoading,
    hasMore,
    isLoading,
    resultState,
    error,
    scopeHealth,
    selectedIds,
    selectedIdList,
    selectedFiles,
    selection,
    ownsSingleFileSelection,
    focusedId,
    detail,
    selectionSummary,
    isInspectorLoading,
    inspectorError,
    tags,
    savedViews,
    activeViewId,
    presentationEntryAt,
    setScope,
    setQueryScope,
    chooseFolders,
    loadNextPage,
    refreshResults,
    loadDetail,
    commitDetailIfCurrent,
    loadSelectionSummary,
    clearInspector,
    clearSelection,
    setExplicitSelection,
    setFocusedId,
    moveFocus,
    toggleSelection,
    selectAllMatching,
    selectionContainsFileId,
    loadTags,
    mutateTags,
    loadSavedViews,
    setActiveViewId,
    ...queryController,
    clearExecutionIntent,
    refreshPreviewsForFiles,
    refreshPreviewsForSelection,
    setPreviewResult
  };
}

/**
 * Adapt one supplied Query V2 row on demand. Keeping this as an indexed
 * helper makes the window-only contract explicit and testable without ever
 * mapping a logical collection into a second presentation model.
 */
export function libraryPresentationEntryAt(
  files: readonly FileLibrarySummary[],
  index: number
): LibraryPresentationEntry | undefined {
  const summary = files[index];
  return summary ? adaptLibrarySummary(summary) : undefined;
}
