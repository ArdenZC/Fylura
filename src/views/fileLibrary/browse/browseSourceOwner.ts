import { usePane } from "../workbench/PaneContext";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { Translator } from "../../../types/ui";
import type {
  BrowseEntry,
  BrowseEntryRef,
  BrowseOpenResponse,
  BrowsePage,
  BrowsePathRef,
  BrowseQueryEntryKind,
  BrowseQuerySpecV1,
  LocationAvailability,
  LocationDescriptor,
  LocationRef,
  NavigationTarget,
  WorkspacePlatform
} from "../../../types/fileWorkspace";
import type { FileLibraryExperienceController, FileLibraryExperienceState } from "../fileLibraryExperience";
import { adaptBrowseEntry, adaptBrowsePageCollection } from "../presentation/adapters";
import type {
  BrowsePresentationCollectionContext,
  BrowsePresentationEntry
} from "../presentation/contracts";

export const BROWSE_PAGE_SIZE = 32;
export const QUERY_SCAN_PAGE_BATCH = 8;

export type BrowseEnumerationState = "idle" | "loading" | "loading_more" | "partial" | "complete" | "failed";
export type BrowseLocationState = "idle" | "loading" | "ready" | "failed";
export type BrowseChangeState = "unavailable" | "starting" | "watching" | "checking" | "refreshing" | "failed";
export type BrowseSelectionIntent = "replace" | "toggle" | "range";

export interface BrowseBreadcrumb {
  readonly sessionId: string;
  readonly pathRef: BrowsePathRef;
  readonly label: string;
}

export interface BrowseSourceOwner {
  readonly locations: readonly LocationDescriptor[];
  readonly locationState: BrowseLocationState;
  readonly locationError: boolean;
  readonly admissionError: boolean;
  readonly admissionLoading: boolean;
  readonly enumerationState: BrowseEnumerationState;
  readonly enumerationError: boolean;
  readonly showLocationPicker: boolean;
  readonly browse: BrowseOpenResponse | null;
  readonly target: Extract<NavigationTarget, { kind: "browse" }> | null;
  readonly sessionId: string | null;
  readonly currentPathRef: BrowsePathRef | null;
  readonly breadcrumbs: readonly BrowseBreadcrumb[];
  readonly entries: readonly BrowsePresentationEntry[];
  readonly collection: BrowsePresentationCollectionContext | null;
  readonly query: BrowseQuerySpecV1;
  readonly queryText: string;
  readonly queryEntryKind: BrowseQueryEntryKind;
  readonly isQueryActive: boolean;
  readonly browseSortAvailable: true;
  readonly skippedEntries: number;
  readonly setSort: (sort: BrowseQuerySpecV1["sort"]) => void;
  readonly selectIds: (ids: readonly string[]) => void;
  readonly completion: BrowsePage["completion"] | null;
  readonly knownCount: number | null;
  readonly hasMore: boolean;
  readonly loadedCount: number;
  readonly selectedIds: ReadonlySet<string>;
  readonly selectedEntryRefs: readonly BrowseEntryRef[];
  readonly focusedId: string | null;
  readonly selectedCount: number;
  readonly pendingChange: FileLibraryExperienceState["workspace"]["pendingChange"];
  readonly canWatch: boolean;
  readonly changeState: BrowseChangeState;
  readonly changeError: boolean;
  readonly loadLocations: () => Promise<void>;
  readonly openLocationPicker: () => void;
  readonly chooseFolder: () => Promise<boolean>;
  readonly activateLocation: (location: LocationDescriptor) => Promise<boolean>;
  readonly refreshEnumeration: () => Promise<void>;
  readonly loadNextPage: () => Promise<void>;
  readonly navigateInto: (entry: BrowsePresentationEntry) => boolean;
  readonly navigateToBreadcrumb: (breadcrumb: BrowseBreadcrumb) => boolean;
  readonly selectEntry: (entryId: string, intent: BrowseSelectionIntent) => void;
  readonly selectAllLoaded: () => void;
  readonly clearSelection: () => void;
  readonly setFocusedId: (entryId: string | null) => void;
  /** Moves only the current-generation loaded focus, requesting one owner page at the edge. */
  readonly moveFocus: (direction: "previous" | "next") => Promise<boolean>;
  readonly setQueryText: (text: string) => void;
  readonly setQueryEntryKind: (kind: BrowseQueryEntryKind) => void;
}

type BrowseController = Pick<FileLibraryExperienceController, "browseLocation" | "openBrowse" | "navigate" | "setBrowseQuery"> & {
  workspace: FileLibraryExperienceController["workspace"];
};

export function browseBreadcrumbKey(sessionId: string, pathRef: BrowsePathRef) {
  return `${sessionId}:${pathRef.id}`;
}

export function createBrowseBreadcrumb(
  sessionId: string,
  pathRef: BrowsePathRef,
  label: string
): BrowseBreadcrumb {
  return { sessionId, pathRef, label };
}

export function appendBrowseBreadcrumb(
  chain: readonly BrowseBreadcrumb[],
  breadcrumb: BrowseBreadcrumb
): BrowseBreadcrumb[] {
  const existingIndex = chain.findIndex((item) =>
    item.sessionId === breadcrumb.sessionId && item.pathRef.id === breadcrumb.pathRef.id
  );
  if (existingIndex >= 0) return [...chain.slice(0, existingIndex + 1)];
  return [...chain, breadcrumb];
}

export function browseBreadcrumbChainForPath(
  chains: ReadonlyMap<string, readonly BrowseBreadcrumb[]>,
  sessionId: string,
  pathRef: BrowsePathRef
) {
  const chain = chains.get(browseBreadcrumbKey(sessionId, pathRef));
  return chain === undefined ? null : [...chain];
}

export function browseEnumerationStateForPage(page: Pick<BrowsePage, "completion">) {
  return page.completion === "complete" ? "complete" : "partial";
}

export interface BrowseNextPageScanResult {
  readonly pages: readonly BrowsePage[];
  readonly entries: readonly BrowseEntry[];
  readonly stale: boolean;
  readonly failed: boolean;
}

/**
 * Advances the owning Browse enumeration only far enough to find the next
 * visible query page. It deliberately retains no sibling window of its own.
 */
export async function scanNextBrowsePages({
  loadPage,
  scanEmptyPages,
  isCurrent,
  isPageAcceptable = () => true
}: {
  loadPage: () => Promise<BrowsePage | null>;
  scanEmptyPages: boolean;
  isCurrent: () => boolean;
  isPageAcceptable?: (page: BrowsePage) => boolean;
}): Promise<BrowseNextPageScanResult> {
  const pages: BrowsePage[] = [];
  const entries: BrowseEntry[] = [];
  const maxPages = scanEmptyPages ? QUERY_SCAN_PAGE_BATCH : 1;
  for (let turn = 0; turn < maxPages; turn += 1) {
    if (!isCurrent()) return { pages, entries, stale: true, failed: false };
    const page = await loadPage();
    if (!isCurrent()) return { pages, entries, stale: true, failed: false };
    if (page === null) return { pages, entries, stale: false, failed: true };
    if (!isPageAcceptable(page)) return { pages, entries, stale: true, failed: false };
    pages.push(page);
    entries.push(...page.entries);
    if (!scanEmptyPages || page.entries.length > 0 || page.completion === "complete") break;
  }
  return { pages, entries, stale: false, failed: false };
}

export function useBrowseSourceOwner({
  controller,
  state,
  t,
  platform = "browser"
}: {
  controller: BrowseController;
  state: FileLibraryExperienceState;
  t: Translator;
  platform?: NodeJS.Platform | "browser";
}): BrowseSourceOwner {
  const visible = usePane()?.visible ?? true;
  useEffect(() => { void controller.workspace.setObservationActive?.(visible); }, [controller.workspace, visible]);
  const locations = state.workspace.locations;
  const browse = state.workspace.browse;
  const currentTarget = state.workspace.session.currentTarget;
  const target = currentTarget?.kind === "browse" ? currentTarget : null;
  const sessionId = target?.location.kind === "ephemeral" ? target.location.browseSessionId : null;
  const currentPathRef = target?.pathRef ?? null;
  const targetKey = target === null
    ? null
    : `${sessionId ?? "managed"}:${target.pathRef.id}`;
  const canWatch = browse?.location.capabilities.canWatch === true;
  const [locationState, setLocationState] = useState<BrowseLocationState>("idle");
  const [locationError, setLocationError] = useState(false);
  const [admissionError, setAdmissionError] = useState(false);
  const [admissionLoading, setAdmissionLoading] = useState(false);
  const admissionLoadingRef = useRef(false);
  const [enumerationState, setEnumerationState] = useState<BrowseEnumerationState>("idle");
  const [enumerationError, setEnumerationError] = useState(false);
  const [showLocationPicker, setShowLocationPicker] = useState(target === null);
  const [activePage, setActivePage] = useState<BrowsePage | null>(null);
  const [entries, setEntries] = useState<BrowsePresentationEntry[]>([]);
  const initialBrowseQuery = state.workspace.session.presentation.browseQuery ?? { text: null, entryKind: "all" as const };
  const [query, setQuery] = useState<BrowseQuerySpecV1>(
    () => initialBrowseQuery
  );
  const queryRef = useRef(query);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [changeState, setChangeState] = useState<BrowseChangeState>("unavailable");
  const [changeError, setChangeError] = useState(false);
  const generationRef = useRef(0);
  const activeTargetKeyRef = useRef<string | null>(null);
  const [queryTargetKey, setQueryTargetKey] = useState<string | null>(null);
  const activeEnumerationKeyRef = useRef<string | null>(null);
  const consumedAdmissionPageRef = useRef<BrowsePage | null>(null);
  const activeChangeTargetKeyRef = useRef<string | null>(null);
  const selectionAnchorRef = useRef<string | null>(null);
  const locationLoadStartedRef = useRef(false);
  const breadcrumbChainsRef = useRef<Map<string, readonly BrowseBreadcrumb[]>>(new Map());
  const breadcrumbSessionRef = useRef<string | null>(null);
  const entriesRef = useRef(entries);
  const focusedIdRef = useRef(focusedId);
  const hasMoreRef = useRef(false);
  entriesRef.current = entries;
  focusedIdRef.current = focusedId;
  hasMoreRef.current = activePage?.nextCursor !== undefined;

  useEffect(() => {
    if (breadcrumbSessionRef.current === sessionId) return;
    breadcrumbChainsRef.current.clear();
    breadcrumbSessionRef.current = sessionId;
  }, [sessionId]);

  const loadLocations = useCallback(async () => {
    setLocationState("loading");
    setLocationError(false);
    setAdmissionError(false);
    locationLoadStartedRef.current = true;
    try {
      const loaded = await controller.workspace.loadLocations();
      if (loaded === null) {
        setLocationState("failed");
        setLocationError(true);
        return;
      }
      setLocationState("ready");
    } catch {
      setLocationState("failed");
      setLocationError(true);
    }
  }, [controller.workspace]);

  useEffect(() => {
    if (state.mode !== "browse") return;
    if (locations.length > 0) {
      setLocationState("ready");
      setLocationError(false);
      return;
    }
    if (locationState === "idle" && !locationLoadStartedRef.current) void loadLocations();
  }, [loadLocations, locationState, locations.length, state.mode]);

  const openLocationPicker = useCallback(() => {
    setShowLocationPicker(true);
    setAdmissionError(false);
    if (locationState === "idle" && !locationLoadStartedRef.current) void loadLocations();
  }, [loadLocations, locationState]);

  const chooseFolder = useCallback(async () => {
    if (admissionLoadingRef.current) return false;
    const workspacePlatform = workspacePlatformForRuntime(platform);
    if (workspacePlatform === null) {
      setAdmissionError(true);
      return false;
    }

    admissionLoadingRef.current = true;
    setAdmissionLoading(true);
    setAdmissionError(false);
    setLocationError(false);
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("browseChooseFolderTitle")
      });
      const selectedPath = Array.isArray(selected) ? selected[0] : selected;
      if (typeof selectedPath !== "string" || selectedPath.trim().length === 0) return false;

      const response = await controller.openBrowse({
        platform: workspacePlatform,
        routingHint: selectedPath,
        displayHint: displayNameFromRoutingHint(selectedPath)
      });
      if (response === null) {
        setAdmissionError(true);
        return false;
      }
      setShowLocationPicker(false);
      return true;
    } catch {
      setAdmissionError(true);
      return false;
    } finally {
      admissionLoadingRef.current = false;
      setAdmissionLoading(false);
    }
  }, [controller, platform, t]);

  const activateLocation = useCallback(async (location: LocationDescriptor) => {
    if (!isActivatableLocation(location)) return false;
    if (admissionLoadingRef.current) return false;
    admissionLoadingRef.current = true;
    setAdmissionLoading(true);
    setLocationError(false);
    setAdmissionError(false);
    try {
      const response = await controller.browseLocation(location.ref);
      if (response === null) {
        setAdmissionError(true);
        return false;
      }
      setShowLocationPicker(false);
      return true;
    } catch {
      setAdmissionError(true);
      return false;
    } finally {
      admissionLoadingRef.current = false;
      setAdmissionLoading(false);
    }
  }, [controller]);

  const mergePage = useCallback((page: BrowsePage, generation: number, expectedTargetKey: string) => {
    if (generationRef.current !== generation || activeTargetKeyRef.current !== expectedTargetKey) return false;
    setActivePage(page);
    setEntries((current) => mergeBrowseEntries(current, page.entries));
    setEnumerationState(browseEnumerationStateForPage(page));
    setEnumerationError(false);
    return true;
  }, []);

  const prepareEnumerationPublication = useCallback(() => {
    const generation = generationRef.current + 1;
    generationRef.current = generation;
    setEnumerationState("loading");
    setEnumerationError(false);
    setActivePage(null);
    setEntries([]);
    setSelectedIds(new Set());
    setFocusedId(null);
    selectionAnchorRef.current = null;
    return generation;
  }, []);

  const beginEnumeration = useCallback(async (
    pathRef: BrowsePathRef,
    expectedTargetKey: string,
    expectedQuery: BrowseQuerySpecV1
  ) => {
    const generation = prepareEnumerationPublication();
    try {
      const page = await controller.workspace.startEnumeration(
        pathRef,
        `w2-04-${Date.now()}-${generation}`,
        BROWSE_PAGE_SIZE,
        expectedQuery
      );
      if (page === null) {
        if (generationRef.current === generation && activeTargetKeyRef.current === expectedTargetKey) {
          setEnumerationState("failed");
          setEnumerationError(true);
        }
        return;
      }
      mergePage(page, generation, expectedTargetKey);
    } catch {
      if (generationRef.current !== generation || activeTargetKeyRef.current !== expectedTargetKey) return;
      setEnumerationState("failed");
      setEnumerationError(true);
    }
  }, [controller.workspace, mergePage, prepareEnumerationPublication]);

  useEffect(() => {
    if (queryTargetKey === targetKey) return;
    setQueryTargetKey(targetKey);
    // Never leave rows or selection from a different target visible during admission failure.
    generationRef.current++;
    setEntries([]); setActivePage(null); setSelectedIds(new Set()); setFocusedId(null);
    setEnumerationState("idle");
    // An admitted target can also arrive from Places, search or navigation.
    // Close the picker only for a new target; opening it on this target stays local.
    if (targetKey !== null) setShowLocationPicker(false);
    activeTargetKeyRef.current = targetKey;
    activeEnumerationKeyRef.current = null;
    const restoredQuery = state.workspace.session.presentation.browseQuery ?? { text: null, entryKind: "all" as const };
    queryRef.current = restoredQuery;
    setQuery(restoredQuery);
    if (state.mode !== "browse" || targetKey === null || target === null || browse === null || sessionId === null) return;
    const rootBreadcrumb = createBrowseBreadcrumb(sessionId, browse.rootPathRef, browse.location.displayName);
    const rootKey = browseBreadcrumbKey(sessionId, browse.rootPathRef);
    if (!breadcrumbChainsRef.current.has(rootKey)) {
      breadcrumbChainsRef.current.set(rootKey, [rootBreadcrumb]);
    }
  }, [browse, queryTargetKey, sessionId, state.mode, target, targetKey]);

  useEffect(() => {
    if (!visible || state.workspace.suspended) {
      // A suspended PaneHost may have canceled its in-flight enumeration.
      // Forget its key and invalidate late publications so visibility resume
      // starts a fresh request instead of treating the canceled one as active.
      activeEnumerationKeyRef.current = null;
      generationRef.current++;
      return;
    }
    if (state.mode !== "browse" || targetKey === null || target === null || browse === null || sessionId === null || currentPathRef === null) return;
    if (queryTargetKey !== targetKey) return;
    const enumerationKey = `${targetKey}:${JSON.stringify(query)}`;
    if (activeEnumerationKeyRef.current === enumerationKey) return;
    activeEnumerationKeyRef.current = enumerationKey;
    const initial = browse.initialPage;
    if (initial && consumedAdmissionPageRef.current !== initial && state.workspace.page === initial
      && currentPathRef.id === browse.rootPathRef.id
      && !query.text && query.entryKind === "all" && !query.sort) {
      consumedAdmissionPageRef.current = initial;
      const generation = prepareEnumerationPublication();
      mergePage(initial, generation, targetKey);
      const focus = browse.focusEntryRef;
      const targetEntry = focus && initial.entries.find((entry) =>
        entry.ref.browseSessionId === focus.browseSessionId && entry.ref.entryId === focus.entryId);
      if (targetEntry) {
        setSelectedIds(new Set([targetEntry.ref.entryId]));
        setFocusedId(targetEntry.ref.entryId);
        selectionAnchorRef.current = targetEntry.ref.entryId;
      }
      return;
    }
    void beginEnumeration(currentPathRef, targetKey, query);
  }, [beginEnumeration, browse, currentPathRef, mergePage, prepareEnumerationPublication, query, queryTargetKey, sessionId, state.mode, state.workspace.page, state.workspace.suspended, target, targetKey, visible]);

  useEffect(() => {
    if (!visible) { activeChangeTargetKeyRef.current = null; return; }
    if (state.mode !== "browse" || targetKey === null || target === null || browse === null || sessionId === null || currentPathRef === null) {
      if (targetKey === null) {
        activeChangeTargetKeyRef.current = null;
        setChangeState("unavailable");
        setChangeError(false);
      }
      return;
    }
    if (activeChangeTargetKeyRef.current === targetKey) return;
    activeChangeTargetKeyRef.current = targetKey;
    setChangeError(false);
    if (!canWatch) {
      setChangeState("unavailable");
      return;
    }
    const existingChange = state.workspace.change;
    if (existingChange !== null
      && existingChange.sessionId === sessionId
      && existingChange.pathRef.id === currentPathRef.id) {
      setChangeState("watching");
      return;
    }
    setChangeState("starting");
    void controller.workspace.startChange(currentPathRef).then((response) => {
      if (activeChangeTargetKeyRef.current !== targetKey || activeTargetKeyRef.current !== targetKey) return;
      if (response === null) {
        setChangeState("failed");
        setChangeError(true);
        return;
      }
      setChangeState("watching");
    }).catch(() => {
      if (activeChangeTargetKeyRef.current !== targetKey || activeTargetKeyRef.current !== targetKey) return;
      setChangeState("failed");
      setChangeError(true);
    });
  }, [visible, browse, canWatch, controller.workspace, currentPathRef, sessionId, state.mode, state.workspace.change, target, targetKey]);

  useEffect(() => {
    if (targetKey !== null) return;
    activeTargetKeyRef.current = null;
    activeEnumerationKeyRef.current = null;
    activeChangeTargetKeyRef.current = null;
    setChangeState("unavailable");
    setChangeError(false);
    setActivePage(null);
    setEntries([]);
    setSelectedIds(new Set());
    setFocusedId(null);
    setQuery({ text: null, entryKind: "all" });
    selectionAnchorRef.current = null;
    if (state.mode === "browse") setShowLocationPicker(true);
  }, [state.mode, targetKey]);

  const beginChangeRefresh = useCallback(async (
    expectedTargetKey: string,
    expectedQuery: BrowseQuerySpecV1
  ) => {
    const generation = prepareEnumerationPublication();
    setChangeState("refreshing");
    try {
      const page = await controller.workspace.refreshChange(
        `w2-04-change-refresh-${Date.now()}-${generation}`,
        BROWSE_PAGE_SIZE,
        expectedQuery
      );
      if (page === null) {
        if (generationRef.current === generation && activeTargetKeyRef.current === expectedTargetKey) {
          setEnumerationState("failed");
          setEnumerationError(true);
          setChangeState("failed");
          setChangeError(true);
        }
        return;
      }
      if (!mergePage(page, generation, expectedTargetKey)) return;
      setChangeState("watching");
    } catch {
      if (generationRef.current !== generation || activeTargetKeyRef.current !== expectedTargetKey) return;
      setEnumerationState("failed");
      setEnumerationError(true);
      setChangeState("failed");
      setChangeError(true);
    }
  }, [controller.workspace, mergePage, prepareEnumerationPublication]);

  const refreshEnumeration = useCallback(async () => {
    if (currentPathRef === null || targetKey === null) return;
    const expectedTargetKey = targetKey;
    const expectedPathRef = currentPathRef;
    const monitor = state.workspace.change;
    const monitorMatchesTarget = monitor !== null
      && sessionId !== null
      && monitor.sessionId === sessionId
      && monitor.pathRef.id === expectedPathRef.id;
    if (!monitorMatchesTarget) {
      if (!canWatch) setChangeState("unavailable");
      await beginEnumeration(expectedPathRef, expectedTargetKey, query);
      return;
    }

    setChangeState("checking");
    setChangeError(false);
    try {
      const pending = await controller.workspace.readPendingChange();
      if (activeTargetKeyRef.current !== expectedTargetKey) return;
      if (pending !== null) {
        await beginChangeRefresh(expectedTargetKey, query);
        return;
      }
      setChangeState("watching");
      await beginEnumeration(expectedPathRef, expectedTargetKey, query);
    } catch {
      if (activeTargetKeyRef.current !== expectedTargetKey) return;
      setChangeState("failed");
      setChangeError(true);
      await beginEnumeration(expectedPathRef, expectedTargetKey, query);
    }
  }, [beginChangeRefresh, beginEnumeration, canWatch, controller.workspace, currentPathRef, query, sessionId, state.workspace.change, targetKey]);

  const isQueryActive = (query.text?.trim().length ?? 0) > 0 || query.entryKind !== "all" || query.sort !== undefined;

  const loadNextPageEntries = useCallback(async (
    scanEmptyQueryPages = isQueryActive
  ): Promise<readonly BrowsePresentationEntry[]> => {
    if (activePage?.nextCursor === undefined || enumerationState === "loading" || enumerationState === "loading_more") return [];
    const generation = generationRef.current;
    const expectedTargetKey = targetKey;
    const expectedEnumerationId = activePage.enumerationId;
    const expectedSessionId = activePage.sessionId;
    if (expectedTargetKey === null) return [];
    setEnumerationState("loading_more");
    try {
      const scan = await scanNextBrowsePages({
        loadPage: () => controller.workspace.nextPage(BROWSE_PAGE_SIZE),
        scanEmptyPages: scanEmptyQueryPages,
        isCurrent: () => generationRef.current === generation
          && activeTargetKeyRef.current === expectedTargetKey,
        isPageAcceptable: (page) => page.enumerationId === expectedEnumerationId
          && page.sessionId === expectedSessionId
      });
      if (scan.stale) return [];
      if (scan.failed) {
        setEnumerationState("failed");
        setEnumerationError(true);
        return [];
      }
      for (const page of scan.pages) {
        if (!mergePage(page, generation, expectedTargetKey)) return [];
      }
      return scan.entries.map(adaptBrowseEntry);
    } catch {
      if (generationRef.current === generation && activeTargetKeyRef.current === expectedTargetKey) {
        setEnumerationState("failed");
        setEnumerationError(true);
      }
      return [];
    }
  }, [activePage, controller.workspace, enumerationState, isQueryActive, mergePage, targetKey]);

  const loadNextPage = useCallback(async () => {
    await loadNextPageEntries(isQueryActive);
  }, [isQueryActive, loadNextPageEntries]);

  const loadNextQueryPage = useCallback(async () => {
    await loadNextPageEntries(true);
  }, [loadNextPageEntries]);

  const moveFocus = useCallback(async (direction: "previous" | "next") => {
    const currentEntries = entriesRef.current;
    const currentId = focusedIdRef.current;
    const currentIndex = currentEntries.findIndex((entry) => entry.entryRef.entryId === currentId);
    if (currentIndex < 0) return false;

    if (direction === "previous" && currentIndex > 0) {
      const previous = currentEntries[currentIndex - 1];
      if (previous === undefined) return false;
      setFocusedId(previous.entryRef.entryId);
      return true;
    }

    if (direction === "next" && currentIndex + 1 < currentEntries.length) {
      const next = currentEntries[currentIndex + 1];
      if (next === undefined) return false;
      setFocusedId(next.entryRef.entryId);
      return true;
    }

    if (direction !== "next" || !hasMoreRef.current) return false;
    const expectedGeneration = generationRef.current;
    const expectedTargetKey = activeTargetKeyRef.current;
    const expectedFocusedId = currentId;
    const newlyLoaded = await loadNextPageEntries(isQueryActive);
    if (generationRef.current !== expectedGeneration
      || activeTargetKeyRef.current !== expectedTargetKey
      || focusedIdRef.current !== expectedFocusedId) {
      return false;
    }
    const next = newlyLoaded[0] ?? entriesRef.current[currentIndex + 1];
    if (next === undefined || next.entryRef.browseSessionId !== sessionId) return false;
    setFocusedId(next.entryRef.entryId);
    return true;
  }, [isQueryActive, loadNextPageEntries, sessionId]);

  useEffect(() => {
    const emptyPartialQuery = isQueryActive
      && enumerationState === "partial"
      && entries.length === 0
      && activePage?.nextCursor !== undefined;
    if (!visible || state.mode !== "browse" || !emptyPartialQuery) return;

    let timer: number | null = window.setTimeout(() => {
      timer = null;
      void loadNextQueryPage();
    }, 0);
    return () => {
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [visible, activePage?.nextCursor, entries.length, enumerationState, isQueryActive, loadNextQueryPage, state.mode]);

  const navigateInto = useCallback((entry: BrowsePresentationEntry) => {
    if (entry.entryKind !== "directory" || entry.pathRef === undefined || target === null || sessionId === null) return false;
    const currentPath = currentPathRef ?? browse?.rootPathRef;
    if (currentPath === undefined || browse === null) return false;
    const currentChain = browseBreadcrumbChainForPath(
      breadcrumbChainsRef.current,
      sessionId,
      currentPath
    ) ?? [createBrowseBreadcrumb(
      sessionId,
      currentPath,
      currentPath.id === browse.rootPathRef.id ? browse.location.displayName : t("browseCurrentFolder")
    )];
    const childBreadcrumb = createBrowseBreadcrumb(sessionId, entry.pathRef, entry.displayName);
    breadcrumbChainsRef.current.set(
      browseBreadcrumbKey(sessionId, entry.pathRef),
      appendBrowseBreadcrumb(currentChain, childBreadcrumb)
    );
    return controller.navigate({
      kind: "browse",
      location: target.location,
      pathRef: entry.pathRef
    });
  }, [browse, controller, currentPathRef, sessionId, t, target]);

  const breadcrumbs = useMemo(() => {
    if (target === null || sessionId === null || browse === null) return [];
    const projected = browseBreadcrumbChainForPath(
      breadcrumbChainsRef.current,
      sessionId,
      target.pathRef
    );
    if (projected !== null) return projected;
    return [createBrowseBreadcrumb(
      sessionId,
      target.pathRef,
      target.pathRef.id === browse.rootPathRef.id ? browse.location.displayName : t("browseCurrentFolder")
    )];
  }, [browse, sessionId, t, target]);

  const navigateToBreadcrumb = useCallback((breadcrumb: BrowseBreadcrumb) => {
    if (target === null || sessionId === null || breadcrumb.sessionId !== sessionId) return false;
    const chain = browseBreadcrumbChainForPath(
      breadcrumbChainsRef.current,
      sessionId,
      breadcrumb.pathRef
    );
    if (chain === null || !chain.some((item) =>
      item.sessionId === breadcrumb.sessionId && item.pathRef.id === breadcrumb.pathRef.id
    )) return false;
    breadcrumbChainsRef.current.set(
      browseBreadcrumbKey(sessionId, breadcrumb.pathRef),
      chain.slice(0, chain.findIndex((item) => item.pathRef.id === breadcrumb.pathRef.id) + 1)
    );
    return controller.navigate({ kind: "browse", location: target.location, pathRef: breadcrumb.pathRef });
  }, [controller, sessionId, target]);

  const selectEntry = useCallback((entryId: string, intent: BrowseSelectionIntent) => {
    const index = entries.findIndex((entry) => entry.entryRef.entryId === entryId);
    if (index < 0 || sessionId === null || entries[index].entryRef.browseSessionId !== sessionId) return;
    setSelectedIds((current) => {
      if (intent === "replace") return new Set([entryId]);
      if (intent === "toggle") {
        const next = new Set(current);
        if (next.has(entryId)) next.delete(entryId);
        else next.add(entryId);
        return next;
      }
      const anchor = selectionAnchorRef.current;
      const anchorIndex = anchor === null ? index : entries.findIndex((entry) => entry.entryRef.entryId === anchor);
      if (anchorIndex < 0) return new Set([entryId]);
      return new Set(entries.slice(Math.min(anchorIndex, index), Math.max(anchorIndex, index) + 1).map((entry) => entry.entryRef.entryId));
    });
    setFocusedId(entryId);
    if (intent !== "range") selectionAnchorRef.current = entryId;
  }, [entries, sessionId]);

  const selectAllLoaded = useCallback(() => {
    if (sessionId === null) return;
    const loaded = entries
      .filter((entry) => entry.entryRef.browseSessionId === sessionId)
      .map((entry) => entry.entryRef.entryId);
    setSelectedIds(new Set(loaded));
    setFocusedId(loaded[0] ?? null);
    selectionAnchorRef.current = loaded[0] ?? null;
  }, [entries, sessionId]);

  const clearSelection = useCallback(() => {
    setSelectedIds(new Set());
    setFocusedId(null);
    selectionAnchorRef.current = null;
  }, []);

  const collection = useMemo(
    () => activePage === null ? null : adaptBrowsePageCollection(activePage),
    [activePage]
  );
  const selectedEntryRefs = useMemo(
    () => entries.filter((entry) => selectedIds.has(entry.entryRef.entryId)).map((entry) => entry.entryRef),
    [entries, selectedIds]
  );

  const setQueryText = useCallback((text: string) => {
    const nextQuery = { ...queryRef.current, text: text || null };
    queryRef.current = nextQuery;
    setQuery(nextQuery);
    controller.setBrowseQuery(nextQuery);
  }, [controller]);
  const setQueryEntryKind = useCallback((entryKind: BrowseQueryEntryKind) => {
    const nextQuery = { ...queryRef.current, entryKind };
    queryRef.current = nextQuery;
    setQuery(nextQuery);
    controller.setBrowseQuery(nextQuery);
  }, [controller]);

  const setSort = useCallback((sort: BrowseQuerySpecV1["sort"]) => {
    const next = { ...queryRef.current, sort };
    queryRef.current = next; setQuery(next); controller.setBrowseQuery(next);
  }, [controller]);
  const selectIds = useCallback((ids: readonly string[]) => {
    const live = new Set(entries.map((e) => e.entryRef.entryId));
    setSelectedIds(new Set(ids.filter((id) => live.has(id))));
    setFocusedId(ids.find((id) => live.has(id)) ?? null);
  }, [entries]);

  return {
    locations,
    locationState,
    locationError,
    admissionError,
    admissionLoading,
    enumerationState,
    enumerationError,
    showLocationPicker: showLocationPicker || target === null,
    browse,
    target,
    sessionId,
    currentPathRef,
    breadcrumbs,
    entries,
    collection,
    query,
    queryText: query.text ?? "",
    queryEntryKind: query.entryKind,
    isQueryActive,
    browseSortAvailable: true,
    skippedEntries: activePage?.skippedEntries ?? 0,
    setSort,
    selectIds,
    completion: activePage?.completion ?? null,
    knownCount: activePage?.completion === "complete" && typeof activePage.knownCount === "number" ? activePage.knownCount : null,
    hasMore: activePage?.nextCursor !== undefined,
    loadedCount: entries.length,
    selectedIds,
    selectedEntryRefs,
    focusedId,
    selectedCount: selectedIds.size,
    pendingChange: state.workspace.pendingChange,
    canWatch,
    changeState,
    changeError,
    loadLocations,
    openLocationPicker,
    chooseFolder,
    activateLocation,
    refreshEnumeration,
    loadNextPage,
    navigateInto,
    navigateToBreadcrumb,
    selectEntry,
    selectAllLoaded,
    clearSelection,
    setFocusedId,
    moveFocus,
    setQueryText,
    setQueryEntryKind
  } satisfies BrowseSourceOwner;
}

export function mergeBrowseEntries(
  current: readonly BrowsePresentationEntry[],
  incoming: readonly BrowseEntry[]
): BrowsePresentationEntry[] {
  const seen = new Set(current.map((entry) => `${entry.entryRef.browseSessionId}:${entry.entryRef.entryId}`));
  const next = [...current];
  for (const entry of incoming) {
    const key = `${entry.ref.browseSessionId}:${entry.ref.entryId}`;
    if (seen.has(key)) continue;
    seen.add(key);
    next.push(adaptBrowseEntry(entry));
  }
  return next;
}

export function isActivatableLocation(location: LocationDescriptor) {
  return location.availability === "available" && location.capabilities.canBrowse;
}

function workspacePlatformForRuntime(platform: NodeJS.Platform | "browser"): WorkspacePlatform | null {
  if (platform === "win32") return "windows";
  if (platform === "darwin") return "macos";
  return null;
}

function displayNameFromRoutingHint(routingHint: string) {
  const normalized = routingHint.replace(/[\\/]+$/, "");
  const segments = normalized.split(/[\\/]/).filter(Boolean);
  return segments.at(-1) ?? normalized;
}

export function locationAvailabilityLabel(availability: LocationAvailability, t: Translator) {
  switch (availability) {
    case "available": return t("browseLocationReady");
    case "permission_denied": return t("browseLocationPermission");
    case "offline": return t("browseLocationOffline");
    case "disconnected": return t("browseLocationDisconnected");
    case "authentication_required": return t("browseLocationAuthentication");
    case "not_found": return t("browseLocationNotFound");
    case "unknown": return t("browseLocationUnknown");
    default: return t("browseLocationUnavailable");
  }
}

export function locationRefSessionId(ref: LocationRef) {
  return ref.kind === "ephemeral" ? ref.browseSessionId : null;
}
