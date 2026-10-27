import { usePaneProjection, usePane } from "../../fileLibrary/workbench/PaneContext";
import { useCallback, useEffect, useRef, useState } from "react";
import { useDebounce as useDebouncedValue } from "../../../hooks/useDebounce";
import {
  cloneFileQuerySpec,
  emptyFileQueryFilters,
  resolveLegacyLibraryScope,
  useFileLibraryQueryStore
} from "../../../store/useFileLibraryV2Store";
import type { FileQueryFiltersV2, FileQuerySpecV2, LibrarySavedView, LibraryScope } from "../../../types/domain";

interface VaultQueryControllerInput {
  legacyScope: LibraryScope;
  querySpec: FileQuerySpecV2;
  setQuerySpec: (spec: FileQuerySpecV2) => void;
  loadFirstPage: () => Promise<void>;
  clearResults: () => void;
  onError: (error: unknown) => void;
  savedViews: LibrarySavedView[];
  activeViewId: string | null;
  setActiveViewId: (id: string | null) => void;
  clearSelection: () => void;
  clearSelectionOnMount?: boolean;
}

export function useVaultQueryController({
  legacyScope,
  querySpec,
  setQuerySpec,
  loadFirstPage,
  clearResults,
  onError,
  savedViews,
  activeViewId,
  setActiveViewId,
  clearSelection,
  clearSelectionOnMount = true
}: VaultQueryControllerInput) {
  const { useFileLibraryQueryStore } = usePaneProjection();
  const visible = usePane()?.visible ?? true;
  const lastLoadedQuery = useRef("");
  const [librarySearch, setLibrarySearch] = useState(() => querySpec.text ?? "");
  const [scopeReady, setScopeReady] = useState(false);
  const debouncedSearchQuery = useDebouncedValue(librarySearch, 300);
  const pendingSavedViewQuerySignature = useRef<string | null>(null);
  const previousSelectionBoundary = useRef<{ querySpecSignature: string; activeViewId: string | null } | null>(null);
  const scopeSignature = `${legacyScope.kind}:${legacyScope.kind === "all" ? "" : `${legacyScope.roots.join("\n")}:${legacyScope.kind === "current_scan" ? legacyScope.scanSessionId ?? "" : ""}`}`;
  const querySpecSignature = JSON.stringify(querySpec);
  const isEmptyCurrentScanScope = legacyScope.kind === "current_scan" && legacyScope.roots.length === 0 && !legacyScope.scanSessionId;

  useEffect(() => {
    let cancelled = false;
    setScopeReady(false);
    if (isEmptyCurrentScanScope) {
      lastLoadedQuery.current = "";
      clearResults();
      setScopeReady(true);
      return () => { cancelled = true; };
    }
    void resolveLegacyLibraryScope(legacyScope).then((scope) => {
      if (cancelled) return;
      const current = useFileLibraryQueryStore.getState().spec;
      setQuerySpec({ ...current, scope });
      setScopeReady(true);
    }).catch((error) => {
      if (cancelled) return;
      setScopeReady(true);
      onError(error);
    });
    return () => { cancelled = true; };
  }, [clearResults, isEmptyCurrentScanScope, legacyScope, onError, scopeSignature, setQuerySpec]);

  useEffect(() => {
    if (!visible || !scopeReady || isEmptyCurrentScanScope) return;
    const waitingForSavedViewSearch = pendingSavedViewQuerySignature.current === querySpecSignature
      && debouncedSearchQuery.trim() !== (querySpec.text ?? "").trim();
    if (waitingForSavedViewSearch) return;
    if (pendingSavedViewQuerySignature.current) pendingSavedViewQuerySignature.current = null;
    const spec = cloneFileQuerySpec({
      ...querySpec,
      text: debouncedSearchQuery.trim() || null,
      sort: querySpec.sort.kind === "relevance" && !debouncedSearchQuery.trim()
        ? { kind: "modified", direction: "desc" }
        : querySpec.sort
    });
    const nextSignature = JSON.stringify(spec);
    if (nextSignature !== querySpecSignature) {
      setQuerySpec(spec);
      return;
    }
    if (lastLoadedQuery.current === querySpecSignature) return;
    lastLoadedQuery.current = querySpecSignature;
    void loadFirstPage().catch(() => undefined);
  }, [visible, debouncedSearchQuery, isEmptyCurrentScanScope, loadFirstPage, querySpec, querySpecSignature, scopeReady, setQuerySpec]);

  useEffect(() => {
    const previous = previousSelectionBoundary.current;
    const selectionBoundaryChanged = previous !== null
      && (previous.querySpecSignature !== querySpecSignature || previous.activeViewId !== activeViewId);
    if (clearSelectionOnMount || selectionBoundaryChanged) clearSelection();
    previousSelectionBoundary.current = { querySpecSignature, activeViewId };
    if (!activeViewId) return;
    const activeView = savedViews.find((view) => view.id === activeViewId);
    if (!activeView) {
      setActiveViewId(null);
      return;
    }
    const waitingForSavedViewSearch = pendingSavedViewQuerySignature.current === querySpecSignature
      && debouncedSearchQuery.trim() !== (querySpec.text ?? "").trim();
    if (!waitingForSavedViewSearch && querySpecSignatureForSavedView(activeView.query) !== querySpecSignature) {
      setActiveViewId(null);
    }
  }, [activeViewId, clearSelection, clearSelectionOnMount, debouncedSearchQuery, querySpec.text, querySpecSignature, savedViews, setActiveViewId]);

  const updateFilters = useCallback((value: Partial<FileQueryFiltersV2>) => {
    const current = useFileLibraryQueryStore.getState().spec;
    setQuerySpec({ ...current, filters: { ...current.filters, ...value } });
  }, [setQuerySpec]);

  const clearFilters = useCallback(() => {
    const current = useFileLibraryQueryStore.getState().spec;
    setQuerySpec({ ...current, filters: { ...emptyFileQueryFilters } });
    clearSelection();
  }, [clearSelection, setQuerySpec]);

  const setSort = useCallback((kind: FileQuerySpecV2["sort"]["kind"]) => {
    const current = useFileLibraryQueryStore.getState().spec.sort;
    setQuerySpec({ ...useFileLibraryQueryStore.getState().spec, sort: { kind, direction: current.kind === kind && current.direction === "desc" ? "asc" : "desc" } });
  }, [setQuerySpec]);

  const applySavedView = useCallback((view: LibrarySavedView | null) => {
    pendingSavedViewQuerySignature.current = view ? querySpecSignatureForSavedView(view.query) : null;
    setActiveViewId(view?.id ?? null);
    if (!view) return;
    setQuerySpec(cloneFileQuerySpec(view.query));
    setLibrarySearch(view.query.text ?? "");
  }, [setActiveViewId, setQuerySpec]);

  const handleLibrarySearchChange = useCallback((value: string) => {
    pendingSavedViewQuerySignature.current = null;
    setActiveViewId(null);
    setLibrarySearch(value);
  }, [setActiveViewId]);

  return {
    librarySearch,
    debouncedSearchQuery,
    setLibrarySearch,
    scopeReady,
    isEmptyCurrentScanScope,
    querySpecSignature,
    updateFilters,
    clearFilters,
    setSort,
    applySavedView,
    handleLibrarySearchChange
  };
}

function querySpecSignatureForSavedView(spec: FileQuerySpecV2): string {
  return JSON.stringify(cloneFileQuerySpec(spec));
}
