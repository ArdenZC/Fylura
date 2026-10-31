import { fileWorkspaceApi } from "../../../api/fileWorkspaceApi";
import type {
  BrowsePage,
  BrowsePathRef,
  LocationDescriptor,
  WorkspacePlatform,
} from "../../../types/fileWorkspace";
import type { Translator } from "../../../types/ui";
import { readExpandedTreePaths, writeExpandedTreePaths } from "../workbench/workbenchPersistence";
import { locationAvailabilityLabel } from "../browse/browseSourceOwner";
import type { FileLibraryExperienceController } from "../fileLibraryExperience";
import { ChevronDown, ChevronRight, CircleAlert, Folder, FolderOpen, LocateFixed, LoaderCircle } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { cn } from "../../../utils/tw";

const PAGE_SIZE = 32;
const MAX_TREE_NODES = 200;
const MAX_DEPTH = 12;
const MAX_EXPANDED_NODES = 24;
const MAX_AUTO_LOCATE_ENTRIES = PAGE_SIZE * 64;
const MAX_AUTO_LOCATE_PAGES = 64;
const ROOT_COLLAPSED_MARKER = "@zen-canvas/tree-root-collapsed:v1";
const SAFE_TRASH_DIRECTORY_NAME = ".zen-canvas-trash";

function isSafeTrashNamespacePath(path: string) {
  return path.split(/[\\/]+/).some(
    (segment) => segment.toLocaleLowerCase() === SAFE_TRASH_DIRECTORY_NAME,
  );
}

interface TreeNode {
  key: string;
  pathRef: BrowsePathRef;
  name: string;
  /** Presentation only. Navigation still re-admits through the workspace API. */
  displayPath: string;
  depth: number;
  childKeys: string[];
  expanded: boolean;
  loading: boolean;
  loaded: boolean;
  scannedCount: number;
  nextCursor: string | null;
  hasMore: boolean;
  error: unknown | null;
}

interface TreeSession {
  sessionId: string;
  generation: number;
  retainedPathRefs: Map<string, BrowsePathRef>;
}

function normalizePath(path: string, platform: NodeJS.Platform | "browser") {
  const normalized = path.replaceAll("\\", "/").replace(/\/+$/, "") || "/";
  return platform === "win32" ? normalized.toLocaleLowerCase() : normalized;
}

function childNodeKey(sessionId: string, pathRef: BrowsePathRef) {
  return `${sessionId}:${pathRef.id}`;
}

function parentDisplayPath(path: string) {
  const separator = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (separator < 0) return null;
  if (separator === 2 && path.length >= 3 && path[1] === ":") return path.slice(0, 3);
  return path.slice(0, separator) || path.slice(0, separator + 1);
}

function pathBasename(path: string) {
  const normalized = path.replace(/[\\/]+$/, "");
  const separator = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
  return separator < 0 ? normalized : normalized.slice(separator + 1);
}

function locationDisplayPath(displayName: string, currentPath: string | null, platform: NodeJS.Platform | "browser") {
  // A Browse Location can itself be the active pane directory. Location labels
  // are names, not paths, so retain the already-admitted pane path for display
  // and highlighting when the label matches its final component.
  if (currentPath === null) return displayName;
  if (normalizePath(displayName, platform) === normalizePath(currentPath, platform)) return currentPath;
  if (normalizePath(displayName, platform) === normalizePath(pathBasename(currentPath), platform)) return currentPath;
  return displayName;
}

function isAncestorPath(ancestor: string, target: string, platform: NodeJS.Platform | "browser") {
  const normalizedAncestor = normalizePath(ancestor, platform);
  const normalizedTarget = normalizePath(target, platform);
  return normalizedTarget === normalizedAncestor
    || normalizedTarget.startsWith(normalizedAncestor.endsWith("/") ? normalizedAncestor : `${normalizedAncestor}/`);
}

export function describeDirectoryTreeError(error: unknown, t: Translator) {
  const message = error instanceof Error ? error.message : String(error);
  const normalized = message.toLowerCase();
  if (normalized.includes("permission") || normalized.includes("access_denied") || normalized.includes("access denied"))
    return t("fileLibraryNavigationTreePermissionDenied");
  if (normalized.includes("not_found") || normalized.includes("not found") || normalized.includes("no such file"))
    return t("fileLibraryNavigationTreeNotFound");
  if (normalized.includes("disconnected") || normalized.includes("offline") || normalized.includes("not_ready") || normalized.includes("device_not_connected"))
    return t("fileLibraryNavigationTreeDisconnected");
  return t("fileLibraryNavigationTreeUnavailable");
}

export function DirectoryTree({
  controller,
  location,
  currentPath,
  currentTargetKey,
  rootPath,
  paneId,
  platform,
  navigationError,
  onNavigationError,
  t,
}: {
  controller: FileLibraryExperienceController;
  location: LocationDescriptor;
  currentPath: string | null;
  currentTargetKey: string | null;
  /** The current Browse target is the admitted root of this tree session. */
  rootPath: string | null;
  paneId: string;
  platform: NodeJS.Platform | "browser";
  navigationError: unknown | null;
  onNavigationError: (error: unknown | null) => void;
  t: Translator;
}) {
  const [nodes, setNodes] = useState<Map<string, TreeNode>>(() => new Map());
  const nodesRef = useRef(nodes);
  const sessionRef = useRef<TreeSession | null>(null);
  const generationRef = useRef(0);
  const requestSequenceRef = useRef(0);
  const busyGenerationRef = useRef<number | null>(null);
  const locatingRef = useRef(false);
  const locateRequestRef = useRef<string | null>(null);
  const pendingLocateRequestRef = useRef<string | null>(null);
  const currentPathRef = useRef(currentPath);
  currentPathRef.current = currentPath;
  const currentTargetKeyRef = useRef(currentTargetKey);
  currentTargetKeyRef.current = currentTargetKey;
  const rootPathRef = useRef(rootPath);
  rootPathRef.current = rootPath;
  const activeRowRef = useRef<HTMLButtonElement | null>(null);
  const treeItemRefs = useRef(new Map<string, HTMLDivElement>());
  const [focusedNodeKey, setFocusedNodeKey] = useState<string | null>(null);
  const autoExpandedRef = useRef(new Set<string>());
  const locateScannedRef = useRef(0);
  const locatePageCountRef = useRef(0);
  const [sessionLoading, setSessionLoading] = useState(false);
  const [sessionError, setSessionError] = useState<unknown | null>(null);
  const [locateStatus, setLocateStatus] = useState<"not_in_tree" | "unavailable" | "depth_limit" | "node_limit" | "scan_limit" | null>(null);
  const [locateNonce, setLocateNonce] = useState(0);
  const [treeRefreshNonce, setTreeRefreshNonce] = useState(0);
  const [restoreEpoch, setRestoreEpoch] = useState(0);
  const [savedExpandedPaths, setSavedExpandedPaths] = useState<string[]>(() => readExpandedTreePaths(paneId));
  const initialCurrentPathRef = useRef<string | null>(currentPath);
  const initialTargetKeyRef = useRef<string | null>(currentTargetKey);
  const canBrowse = location.availability === "available" && location.capabilities.canBrowse;
  const locationKey = location.ref.kind === "managed"
    ? `managed:${location.ref.scanRootId}`
    : `ephemeral:${location.ref.browseSessionId}:${location.ref.locationId}`;
  const treeBusy = busyGenerationRef.current === sessionRef.current?.generation;

  useEffect(() => {
    // Workbench restore can migrate saved expansion metadata to the live pane id
    // just after the first render. Re-read on pane/navigation admission changes.
    setSavedExpandedPaths(readExpandedTreePaths(paneId));
  }, [currentPath, location.ref, paneId]);

  const publishNodes = useCallback((next: Map<string, TreeNode>) => {
    nodesRef.current = next;
    setNodes(next);
  }, []);

  const updateNode = useCallback((key: string, update: (node: TreeNode) => TreeNode) => {
    const node = nodesRef.current.get(key);
    if (!node) return;
    const next = new Map(nodesRef.current);
    next.set(key, update(node));
    publishNodes(next);
  }, [publishNodes]);

  const persistExpanded = useCallback((nextNodes: Map<string, TreeNode>) => {
    const paths = [...nextNodes.values()]
      .filter((node) => node.expanded)
      .sort((a, b) => a.depth - b.depth)
      .slice(0, MAX_EXPANDED_NODES)
      .map((node) => node.displayPath);
    if ([...nextNodes.values()].some((node) => node.depth === 0 && !node.expanded)) {
      paths.unshift(ROOT_COLLAPSED_MARKER);
      paths.splice(MAX_EXPANDED_NODES);
    }
    setSavedExpandedPaths(paths);
    writeExpandedTreePaths(paneId, paths);
  }, [paneId]);

  const disposeTreeSession = useCallback(async (session: TreeSession) => {
    await Promise.allSettled([...session.retainedPathRefs.values()].map((pathRef) =>
      fileWorkspaceApi.browseReleasePath({ sessionId: session.sessionId, pathRef })
    ));
    await fileWorkspaceApi.browseDispose({ sessionId: session.sessionId }).catch(() => undefined);
  }, []);

  const loadChildren = useCallback(async (key: string, loadMore = false) => {
    const session = sessionRef.current;
    const initialNode = nodesRef.current.get(key);
    if (!session || !initialNode || initialNode.loading || busyGenerationRef.current === session.generation || initialNode.depth >= MAX_DEPTH) return;
    if (nodesRef.current.size - 1 >= MAX_TREE_NODES && !loadMore) return;
    const generation = session.generation;
    busyGenerationRef.current = generation;
    updateNode(key, (node) => ({ ...node, loading: true, error: null, expanded: true }));
    let page: BrowsePage | null = null;
    let scanned = initialNode.scannedCount;
    let hasMore = false;
    let failure: unknown | null = null;
    try {
      page = loadMore && initialNode.nextCursor
        ? await fileWorkspaceApi.browseNextPage({ sessionId: session.sessionId, cursor: initialNode.nextCursor, pageSize: PAGE_SIZE })
        : await fileWorkspaceApi.browseStartEnumeration({
            sessionId: session.sessionId,
            requestId: `tree-${generation}-${++requestSequenceRef.current}`,
            pathRef: initialNode.pathRef,
            pageSize: PAGE_SIZE,
            query: {
              text: null,
              entryKind: "directory",
              sort: { field: "name", direction: "asc", foldersFirst: true },
            },
          });
      if (page !== null) {
        if (generationRef.current !== generation || sessionRef.current !== session) {
          await fileWorkspaceApi.browseReleasePage({ page }).catch(() => undefined);
          page = null;
        } else {
          const currentPage = page;
          page = null;
          if (locatingRef.current) locatePageCountRef.current += 1;
          const next = new Map(nodesRef.current);
          const parent = next.get(key);
          if (!parent) {
            await fileWorkspaceApi.browseReleasePage({ page: currentPage }).catch(() => undefined);
          } else {
            const childKeys = [...parent.childKeys];
            const knownChildKeys = new Set(childKeys);
            let nodeCount = next.size - 1;
            let stale = false;
            for (const entry of currentPage.entries) {
              scanned += 1;
              if (locatingRef.current) locateScannedRef.current += 1;
              if (
                entry.kind !== "directory" ||
                !entry.pathRef ||
                entry.name.toLocaleLowerCase() === SAFE_TRASH_DIRECTORY_NAME ||
                isSafeTrashNamespacePath(entry.displayPath)
              ) continue;
              const childKey = childNodeKey(session.sessionId, entry.pathRef);
              if (parent.depth === 0 && rootPathRef.current === null) {
                const inferredRootPath = parentDisplayPath(entry.displayPath);
                if (inferredRootPath !== null) {
                  const refreshedParent = next.get(key);
                  if (refreshedParent) next.set(key, { ...refreshedParent, displayPath: inferredRootPath });
                }
              }
              if (knownChildKeys.has(childKey) || nodeCount >= MAX_TREE_NODES) continue;
              try {
                if (!session.retainedPathRefs.has(entry.pathRef.id)) {
                  await fileWorkspaceApi.browseRetainPath({ sessionId: session.sessionId, pathRef: entry.pathRef });
                  if (generationRef.current !== generation || sessionRef.current !== session) {
                    await fileWorkspaceApi.browseReleasePath({ sessionId: session.sessionId, pathRef: entry.pathRef }).catch(() => undefined);
                    stale = true;
                    break;
                  }
                  session.retainedPathRefs.set(entry.pathRef.id, entry.pathRef);
                }
              } catch (error) {
                failure = error;
                continue;
              }
              const child: TreeNode = {
                key: childKey,
                pathRef: entry.pathRef,
                name: entry.name,
                displayPath: entry.displayPath,
                depth: parent.depth + 1,
                childKeys: [],
                expanded: false,
                loading: false,
                loaded: false,
                scannedCount: 0,
                nextCursor: null,
                hasMore: false,
                error: null,
              };
              next.set(childKey, child);
              childKeys.push(childKey);
              knownChildKeys.add(childKey);
              nodeCount += 1;
            }
            if (stale || generationRef.current !== generation || sessionRef.current !== session) {
              await fileWorkspaceApi.browseReleasePage({ page: currentPage }).catch(() => undefined);
            } else {
              const cursor = currentPage.nextCursor ?? null;
              hasMore = cursor !== null;
              const updatedParent = next.get(key);
              if (updatedParent) {
                next.set(key, {
                  ...updatedParent,
                  childKeys,
                  scannedCount: scanned,
                  nextCursor: cursor,
                  hasMore,
                  loaded: true,
                  loading: false,
                  error: failure,
                });
              }
              publishNodes(next);
              await fileWorkspaceApi.browseReleasePage({ page: currentPage }).catch(() => undefined);
            }
          }
        }
      }
    } catch (error) {
      failure = error;
      if (page) await fileWorkspaceApi.browseReleasePage({ page }).catch(() => undefined);
    } finally {
      if (busyGenerationRef.current === generation) busyGenerationRef.current = null;
      if (generationRef.current === generation && sessionRef.current === session) {
        updateNode(key, (node) => ({ ...node, loading: false, error: failure ?? node.error }));
      }
    }
  }, [publishNodes, updateNode]);

  useEffect(() => {
    const generation = ++generationRef.current;
    let cancelled = false;
    const oldSession = sessionRef.current;
    sessionRef.current = null;
    if (oldSession) void disposeTreeSession(oldSession);
    publishNodes(new Map());
    autoExpandedRef.current.clear();
    locateRequestRef.current = null;
    pendingLocateRequestRef.current = null;
    setLocateStatus(null);
    setSessionError(null);
    if (!canBrowse) {
      setSessionLoading(false);
      return () => { cancelled = true; };
    }

    setSessionLoading(true);
    void fileWorkspaceApi.locationBrowse({ location: location.ref }).then(async (response) => {
      if (cancelled || generationRef.current !== generation) {
        await fileWorkspaceApi.browseDispose({ sessionId: response.sessionId }).catch(() => undefined);
        return;
      }
      const session: TreeSession = {
        sessionId: response.sessionId,
        generation,
        retainedPathRefs: new Map(),
      };
      sessionRef.current = session;
      const root: TreeNode = {
        key: childNodeKey(session.sessionId, response.rootPathRef),
        pathRef: response.rootPathRef,
        name: response.location.displayName,
        displayPath: rootPath ?? locationDisplayPath(response.location.displayName, currentPath, platform),
        depth: 0,
        childKeys: [],
        expanded: !readExpandedTreePaths(paneId).includes(ROOT_COLLAPSED_MARKER),
        loading: false,
        loaded: false,
        scannedCount: 0,
        nextCursor: null,
        hasMore: false,
        error: null,
      };
      publishNodes(new Map([[root.key, root]]));
      setSessionLoading(false);
      if (root.expanded) await loadChildren(root.key);
    }).catch((error) => {
      if (!cancelled && generationRef.current === generation) {
        setSessionError(error);
        setSessionLoading(false);
      }
    });
    return () => {
      cancelled = true;
      generationRef.current += 1;
      const session = sessionRef.current;
      sessionRef.current = null;
      if (session) void disposeTreeSession(session);
    };
  }, [canBrowse, disposeTreeSession, loadChildren, locationKey, location.displayName, paneId, platform, publishNodes, treeRefreshNonce]);

  useEffect(() => {
    if (rootPath === null) return;
    const root = [...nodesRef.current.values()].find((node) => node.depth === 0);
    if (!root || normalizePath(root.displayPath, platform) === normalizePath(rootPath, platform)) return;
    const next = new Map(nodesRef.current);
    next.set(root.key, { ...root, displayPath: rootPath });
    publishNodes(next);
  }, [platform, publishNodes, rootPath]);

  useEffect(() => {
    if (currentPath === null) {
      locateRequestRef.current = null;
      pendingLocateRequestRef.current = null;
      setLocateStatus(null);
      return;
    }
    const session = sessionRef.current;
    const root = [...nodes.values()].find((node) => node.depth === 0);
    if (!session || !root) return;
    const normalizedCurrentPath = normalizePath(currentPath, platform);
    const requestKey = `${session.sessionId}:${currentTargetKey ?? "unknown"}:${normalizedCurrentPath}`;
    if (locatingRef.current) {
      if (locateRequestRef.current !== requestKey) {
        pendingLocateRequestRef.current = requestKey;
        setLocateStatus(null);
      }
      return;
    }
    if (busyGenerationRef.current === session.generation) return;
    const initialPath = initialCurrentPathRef.current;
    const pathChangedSinceMount = initialPath === null
      || initialTargetKeyRef.current !== currentTargetKey
      || normalizePath(initialPath, platform) !== normalizedCurrentPath;
    if (savedExpandedPaths.includes(ROOT_COLLAPSED_MARKER) && locateNonce === 0 && !pathChangedSinceMount) return;
    if (locateRequestRef.current === requestKey) return;
    locatingRef.current = true;
    locateRequestRef.current = requestKey;
    locateScannedRef.current = 0;
    locatePageCountRef.current = 0;
    setLocateStatus(null);
    const isCurrentLocation = () => generationRef.current === session.generation
      && sessionRef.current === session
      && currentPathRef.current !== null
      && currentTargetKeyRef.current === currentTargetKey
      && normalizePath(currentPathRef.current, platform) === normalizedCurrentPath;
    void (async () => {
      let parentKey = root.key;
      let depth = 0;
      while (depth < MAX_DEPTH) {
        if (!isCurrentLocation()) return;
        let parent = nodesRef.current.get(parentKey);
        if (!parent) return;
        if (!parent.loaded) {
          if (parent.error !== null) {
            setLocateStatus("unavailable");
            return;
          }
          await loadChildren(parentKey);
          if (!isCurrentLocation()) return;
          parent = nodesRef.current.get(parentKey);
          if (!parent || !parent.loaded) return;
        }
        if (parent.error !== null) {
          setLocateStatus("unavailable");
          return;
        }
        if (normalizePath(parent.displayPath, platform) === normalizePath(currentPath, platform)) {
          return;
        }
        const child = parent.childKeys
          .map((childKey) => nodesRef.current.get(childKey))
          .find((candidate): candidate is TreeNode => candidate !== undefined
            && isAncestorPath(candidate.displayPath, currentPath, platform));
        if (!child) {
          if (parent.hasMore && nodesRef.current.size - 1 < MAX_TREE_NODES) {
            if (locateScannedRef.current >= MAX_AUTO_LOCATE_ENTRIES || locatePageCountRef.current >= MAX_AUTO_LOCATE_PAGES) {
              locateRequestRef.current = requestKey;
              setLocateStatus("scan_limit");
              return;
            }
            const previousCursor = parent.nextCursor;
            await loadChildren(parentKey, true);
            if (!isCurrentLocation()) return;
            const refreshed = nodesRef.current.get(parentKey);
            if (refreshed && refreshed.nextCursor !== previousCursor) continue;
            if (refreshed?.hasMore && nodesRef.current.size - 1 >= MAX_TREE_NODES) {
              setLocateStatus("node_limit");
              return;
            }
            if (refreshed?.error !== null && refreshed?.error !== undefined) {
              setLocateStatus("unavailable");
              return;
            }
            if (refreshed?.hasMore && locateScannedRef.current >= MAX_AUTO_LOCATE_ENTRIES) {
              setLocateStatus("scan_limit");
              return;
            }
          }
          setLocateStatus(parent.hasMore ? "node_limit" : "not_in_tree");
          return;
        }
        if (normalizePath(child.displayPath, platform) === normalizePath(currentPath, platform)) {
          return;
        }
        if (!child.expanded) {
          const next = new Map(nodesRef.current);
          const latestChild = next.get(child.key);
          if (latestChild) {
            next.set(child.key, { ...latestChild, expanded: true });
            publishNodes(next);
            persistExpanded(next);
          }
        }
        parentKey = child.key;
        depth += 1;
      }
      setLocateStatus("depth_limit");
    })().finally(() => {
      locatingRef.current = false;
      setRestoreEpoch((epoch) => epoch + 1);
      const pendingRequest = pendingLocateRequestRef.current;
      pendingLocateRequestRef.current = null;
      if (pendingRequest !== null && currentPathRef.current !== null) {
        const latestRequest = `${sessionRef.current?.sessionId ?? ""}:${currentTargetKeyRef.current ?? "unknown"}:${normalizePath(currentPathRef.current, platform)}`;
        if (pendingRequest === latestRequest) setLocateNonce((nonce) => nonce + 1);
      }
    });
  }, [currentPath, currentTargetKey, loadChildren, locateNonce, nodes, persistExpanded, platform, publishNodes, savedExpandedPaths, treeBusy]);

  useEffect(() => {
    if (!savedExpandedPaths.length || treeBusy || locatingRef.current || !sessionRef.current) return;
    const saved = new Set(savedExpandedPaths.map((path) => normalizePath(path, platform)));
    const candidate = [...nodesRef.current.values()].find((node) =>
      node.depth > 0
      && !node.expanded
      && !node.loading
      && saved.has(normalizePath(node.displayPath, platform))
      && !autoExpandedRef.current.has(node.key)
    );
    if (!candidate) return;
    autoExpandedRef.current.add(candidate.key);
    updateNode(candidate.key, (node) => ({ ...node, expanded: true }));
    if (!candidate.loaded) void loadChildren(candidate.key);
  }, [loadChildren, nodes, platform, restoreEpoch, savedExpandedPaths, treeBusy, updateNode]);

  const rootKey = useMemo(() => [...nodes.values()].find((node) => node.depth === 0)?.key ?? null, [nodes]);
  const treeOrder = useMemo(() => {
    const ordered: string[] = [];
    const visit = (key: string) => {
      const node = nodes.get(key);
      if (!node) return;
      ordered.push(key);
      if (node.expanded) node.childKeys.forEach(visit);
    };
    if (rootKey) visit(rootKey);
    return ordered;
  }, [nodes, rootKey]);
  const toggle = useCallback((node: TreeNode) => {
    if (node.loading || busyGenerationRef.current === sessionRef.current?.generation) return;
    const next = new Map(nodesRef.current);
    const current = next.get(node.key);
    if (!current) return;
    const expanded = !current.expanded;
    next.set(node.key, { ...current, expanded });
    publishNodes(next);
    persistExpanded(next);
    if (expanded && !current.loaded) void loadChildren(node.key);
  }, [loadChildren, persistExpanded, publishNodes]);

  const navigateTo = useCallback(async (node: TreeNode) => {
    onNavigationError(null);
    try {
      if (node.depth === 0) {
        const response = await controller.browseLocation(location.ref);
        if (response === null) throw new Error("browse_location_not_admitted");
        return;
      }
      const workspacePlatform: WorkspacePlatform | null = platform === "win32"
        ? "windows"
        : platform === "darwin"
          ? "macos"
          : null;
      if (!workspacePlatform) throw new Error("browse_platform_unavailable");
      const response = await controller.openBrowse({
        platform: workspacePlatform,
        routingHint: node.displayPath,
        displayHint: node.name,
      });
      if (response === null) throw new Error("browse_navigation_not_admitted");
    } catch (error) {
      onNavigationError(error);
    }
  }, [controller, location.ref, onNavigationError, platform]);

  const currentPathKey = currentPath === null ? null : normalizePath(currentPath, platform);
  const activeNodeKey = treeOrder.find((key) => {
    const node = nodes.get(key);
    return currentPathKey !== null && node !== undefined && normalizePath(node.displayPath, platform) === currentPathKey;
  }) ?? null;
  const tabStopKey = focusedNodeKey && treeOrder.includes(focusedNodeKey)
    ? focusedNodeKey
    : activeNodeKey ?? rootKey;
  const focusTreeNode = useCallback((key: string | undefined) => {
    if (!key) return;
    setFocusedNodeKey(key);
    treeItemRefs.current.get(key)?.focus();
  }, []);
  const handleTreeKeyDown = useCallback((event: ReactKeyboardEvent<HTMLDivElement>, node: TreeNode) => {
    const index = treeOrder.indexOf(node.key);
    if (event.key === "ArrowDown") { event.preventDefault(); focusTreeNode(treeOrder[index + 1]); }
    else if (event.key === "ArrowUp") { event.preventDefault(); focusTreeNode(treeOrder[index - 1]); }
    else if (event.key === "Home") { event.preventDefault(); focusTreeNode(treeOrder[0]); }
    else if (event.key === "End") { event.preventDefault(); focusTreeNode(treeOrder.at(-1)); }
    else if (event.key === "ArrowRight") {
      event.preventDefault();
      if (!node.expanded && node.depth < MAX_DEPTH) toggle(node);
      else focusTreeNode(node.childKeys[0]);
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (node.expanded) toggle(node);
      else focusTreeNode([...nodes.values()].find((candidate) => candidate.childKeys.includes(node.key))?.key);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      void navigateTo(node);
    }
  }, [focusTreeNode, navigateTo, nodes, toggle, treeOrder]);

  const rootErrorLabel = sessionError === null ? null : describeDirectoryTreeError(sessionError, t);
  const locateCurrentFolder = useCallback(() => {
    if (currentPath === null || busyGenerationRef.current === sessionRef.current?.generation) return;
    locateRequestRef.current = null;
    pendingLocateRequestRef.current = null;
    setLocateStatus(null);
    setLocateNonce((nonce) => nonce + 1);
    setTreeRefreshNonce((nonce) => nonce + 1);
  }, [currentPath]);

  useEffect(() => {
    if (locateNonce === 0 || sessionLoading) return;
    const frame = window.requestAnimationFrame(() => activeRowRef.current?.scrollIntoView({ block: "nearest" }));
    return () => window.cancelAnimationFrame(frame);
  }, [locateNonce, nodes, sessionLoading]);

  const renderNode = (node: TreeNode): ReactNode => {
    const active = currentPathKey !== null && normalizePath(node.displayPath, platform) === currentPathKey;
    const canExpand = node.depth < MAX_DEPTH;
    const nodeLimitReached = nodes.size - 1 >= MAX_TREE_NODES && node.hasMore;
    return (
      <div
        key={node.key}
        ref={(element) => { if (element) treeItemRefs.current.set(node.key, element); else treeItemRefs.current.delete(node.key); }}
        role="treeitem"
        tabIndex={node.key === tabStopKey ? 0 : -1}
        aria-expanded={canExpand ? node.expanded : undefined}
        aria-current={active ? "location" : undefined}
        onFocus={() => setFocusedNodeKey(node.key)}
        onKeyDown={(event) => handleTreeKeyDown(event, node)}
      >
        <div className="flex min-w-0 items-center gap-1" style={{ paddingInlineStart: `${node.depth * 12}px` }}>
          <button
            type="button"
            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-[var(--zc-text-tertiary)] hover:bg-[var(--zc-surface-hover)] disabled:opacity-50"
            aria-label={node.expanded ? t("fileLibraryNavigationTreeCollapse") : t("fileLibraryNavigationTreeExpand")}
            aria-expanded={node.expanded}
            tabIndex={-1}
            disabled={!canExpand || treeBusy || node.loading}
            onClick={() => toggle(node)}
          >
            {node.loading ? <LoaderCircle size={13} className="animate-spin" aria-hidden="true" />
              : node.expanded ? <ChevronDown size={14} aria-hidden="true" />
                : <ChevronRight size={14} aria-hidden="true" />}
          </button>
          <button
            type="button"
            className={cn("file-library-navigation-item min-w-0 flex-1", active && "is-active")}
            aria-current={active ? "location" : undefined}
            tabIndex={-1}
            ref={active ? activeRowRef : undefined}
            title={node.displayPath}
            onClick={() => navigateTo(node)}
          >
            {node.expanded ? <FolderOpen size={15} aria-hidden="true" /> : <Folder size={15} aria-hidden="true" />}
            <span className="min-w-0 truncate">{node.name}</span>
          </button>
        </div>
        {node.error !== null ? <p className="file-library-navigation-muted flex items-center gap-1" role="status" style={{ paddingInlineStart: `${(node.depth + 1) * 12 + 6}px` }}><CircleAlert size={13} aria-hidden="true" />{describeDirectoryTreeError(node.error, t)}</p> : null}
        {canExpand && node.expanded && node.childKeys.length > 0 ? (
          <div role="group" className="file-library-navigation-list file-library-navigation-sublist">
            {node.childKeys.map((childKey) => {
              const child = nodes.get(childKey);
              return child ? renderNode(child) : null;
            })}
          </div>
        ) : null}
        {node.depth >= MAX_DEPTH ? <p className="file-library-navigation-muted" role="status" style={{ paddingInlineStart: `${(node.depth + 1) * 12 + 6}px` }}>{t("fileLibraryNavigationTreeDepthLimit")}</p> : null}
        {node.expanded && node.loaded && node.childKeys.length === 0 && !node.hasMore && node.error === null ? <p className="file-library-navigation-muted" role="status" style={{ paddingInlineStart: `${(node.depth + 1) * 12 + 6}px` }}>{t("fileLibraryNavigationTreeEmpty")}</p> : null}
        {node.expanded && node.hasMore ? (
          <button
            type="button"
            className="file-library-navigation-item ml-8 min-h-7 text-[11px]"
            disabled={node.loading || nodeLimitReached || treeBusy}
            onClick={() => void loadChildren(node.key, true)}
          >
            {node.loading ? t("fileLibraryNavigationTreeLoading") : nodeLimitReached ? t("fileLibraryNavigationTreeNodeLimit") : t("fileLibraryNavigationTreeLoadMore")}
          </button>
        ) : null}
      </div>
    );
  };

  return (
    <section className="file-library-navigation-section" aria-labelledby={`file-library-navigation-tree-heading-${paneId}`} data-file-library-directory-tree="true">
      <div className="file-library-navigation-section-heading">
        <h3 id={`file-library-navigation-tree-heading-${paneId}`} className="file-library-navigation-section-title">{t("fileLibraryNavigationTree")}</h3>
        <button type="button" className="file-library-navigation-group-toggle !min-h-7 !w-7 !justify-center !p-0" disabled={sessionLoading || !rootKey || currentPath === null || treeBusy} title={t("fileLibraryNavigationTreeLocateCurrent")} aria-label={t("fileLibraryNavigationTreeLocateCurrent")} onClick={locateCurrentFolder}>
          <LocateFixed size={14} aria-hidden="true" />
        </button>
      </div>
      {!canBrowse ? <p className="file-library-navigation-muted" role="status">{locationAvailabilityLabel(location.availability, t)}</p> : null}
      {canBrowse && sessionLoading ? <p className="file-library-navigation-muted flex items-center gap-1" role="status"><LoaderCircle size={13} className="animate-spin" aria-hidden="true" />{t("fileLibraryNavigationTreeLoading")}</p> : null}
      {canBrowse && (rootErrorLabel || navigationError !== null) ? <p className="file-library-navigation-muted flex items-center gap-1" role="status"><CircleAlert size={13} aria-hidden="true" />{rootErrorLabel ?? describeDirectoryTreeError(navigationError, t)}</p> : null}
      {canBrowse && locateStatus ? <p className="file-library-navigation-muted" role="status">{t(locateStatus === "not_in_tree" ? "fileLibraryNavigationTreeCurrentNotLocated" : locateStatus === "unavailable" ? "fileLibraryNavigationTreeUnavailable" : locateStatus === "depth_limit" ? "fileLibraryNavigationTreeDepthLimit" : locateStatus === "scan_limit" ? "fileLibraryNavigationTreeScanLimit" : "fileLibraryNavigationTreeNodeLimit")}</p> : null}
      {canBrowse && !sessionLoading && rootKey ? <div role="tree" aria-label={t("fileLibraryNavigationTree")} className="file-library-navigation-list">
        {nodes.get(rootKey) ? renderNode(nodes.get(rootKey)!) : null}
      </div> : null}
    </section>
  );
}
