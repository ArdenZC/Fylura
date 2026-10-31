import { workbenchTargetKey, type WorkbenchSnapshot } from "./workbenchModel";
import type { WorkspacePlatform } from "../../../types/fileWorkspace";
import {
  parseWorkspaceRestoreMetadata,
  serializeWorkspaceRestoreMetadata,
  type WorkspaceRestoreMetadata,
} from "../../../fileWorkspace/workspaceSession";

const WORKSPACE_STORAGE_KEY = "zen-canvas:file-library-workbench:v1";
const TREE_STORAGE_PREFIX = "zen-canvas:file-library-tree:v1:";
const WORKSPACE_VERSION = 1;
const MAX_TABS = 8;
const MAX_PANES_PER_TAB = 2;
const MAX_PATH_LENGTH = 4096;
const MAX_SNAPSHOT_LENGTH = 128 * 1024;
export const MAX_PERSISTED_TREE_PATHS = 24;

export interface PersistedWorkbenchPane {
  id: string;
  path: string;
  restore: WorkspaceRestoreMetadata | null;
}

export interface PersistedWorkbenchTab {
  id: string;
  panes: PersistedWorkbenchPane[];
  activePaneIndex: number;
  dual: boolean;
}

export interface PersistedWorkbench {
  version: 1;
  tabs: PersistedWorkbenchTab[];
  activeTabIndex: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(value: Record<string, unknown>, allowed: string[]) {
  return Object.keys(value).every((key) => allowed.includes(key));
}

function isStableId(value: unknown): value is string {
  return typeof value === "string" && /^[a-zA-Z0-9_-]{1,48}$/.test(value);
}

function localStorageOrNull(): Storage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

function parsePersistedPane(value: unknown): PersistedWorkbenchPane | null {
  if (
    !isRecord(value) ||
    !hasOnlyKeys(value, ["id", "path", "restore"]) ||
    !isStableId(value.id) ||
    typeof value.path !== "string" ||
    value.path.length > MAX_PATH_LENGTH
  )
    return null;

  let restore: WorkspaceRestoreMetadata | null = null;
  if (value.restore !== null) {
    const parsed = parseWorkspaceRestoreMetadata(value.restore);
    if (!parsed.ok) return null;
    restore = parsed.metadata;
  }

  return { id: value.id, path: value.path, restore };
}

export function parsePersistedWorkbench(value: unknown): PersistedWorkbench | null {
  if (
    !isRecord(value) ||
    !hasOnlyKeys(value, ["version", "tabs", "activeTabIndex"]) ||
    value.version !== WORKSPACE_VERSION ||
    !Array.isArray(value.tabs) ||
    value.tabs.length < 1 ||
    value.tabs.length > MAX_TABS ||
    !Number.isSafeInteger(value.activeTabIndex) ||
    (value.activeTabIndex as number) < 0 ||
    (value.activeTabIndex as number) >= value.tabs.length
  )
    return null;

  const ids = new Set<string>();
  const tabs: PersistedWorkbenchTab[] = [];
  for (const rawTab of value.tabs) {
    if (
      !isRecord(rawTab) ||
      !hasOnlyKeys(rawTab, ["id", "panes", "activePaneIndex", "dual"]) ||
      !isStableId(rawTab.id) ||
      ids.has(rawTab.id) ||
      !Array.isArray(rawTab.panes) ||
      rawTab.panes.length < 1 ||
      rawTab.panes.length > MAX_PANES_PER_TAB ||
      !Number.isSafeInteger(rawTab.activePaneIndex) ||
      (rawTab.activePaneIndex as number) < 0 ||
      (rawTab.activePaneIndex as number) >= rawTab.panes.length ||
      typeof rawTab.dual !== "boolean" ||
      (rawTab.dual && rawTab.panes.length !== 2)
    )
      return null;
    ids.add(rawTab.id);

    const panes: PersistedWorkbenchPane[] = [];
    for (const rawPane of rawTab.panes) {
      const pane = parsePersistedPane(rawPane);
      if (!pane || ids.has(pane.id)) return null;
      ids.add(pane.id);
      panes.push(pane);
    }
    tabs.push({
      id: rawTab.id,
      panes,
      activePaneIndex: rawTab.activePaneIndex as number,
      dual: rawTab.dual,
    });
  }

  return {
    version: WORKSPACE_VERSION,
    tabs,
    activeTabIndex: value.activeTabIndex as number,
  };
}

export function readPersistedWorkbench(
  storage: Storage | null = localStorageOrNull(),
): PersistedWorkbench | null {
  if (!storage) return null;
  try {
    const raw = storage.getItem(WORKSPACE_STORAGE_KEY);
    if (!raw || raw.length > MAX_SNAPSHOT_LENGTH) return null;
    return parsePersistedWorkbench(JSON.parse(raw));
  } catch {
    return null;
  }
}

export function writePersistedWorkbench(
  value: unknown,
  storage: Storage | null = localStorageOrNull(),
) {
  if (!storage) return false;
  const parsed = parsePersistedWorkbench(value);
  if (!parsed) return false;
  try {
    const serialized = JSON.stringify(parsed);
    if (serialized.length > MAX_SNAPSHOT_LENGTH) return false;
    storage.setItem(WORKSPACE_STORAGE_KEY, serialized);
    return true;
  } catch {
    return false;
  }
}

/**
 * Projects navigation intent only. Session refs, Browse pages, transfer
 * tickets, operation previews, and pending mutations are not serializable here.
 */
export function serializeWorkbenchSnapshot(
  snapshot: WorkbenchSnapshot,
  platform: WorkspacePlatform | null,
): PersistedWorkbench | null {
  if (snapshot.tabs.length < 1 || snapshot.tabs.length > MAX_TABS) return null;
  const activeTabIndex = snapshot.tabs.findIndex(
    (tab) => tab.id === snapshot.activeTabId,
  );
  if (activeTabIndex < 0) return null;

  const tabs: PersistedWorkbenchTab[] = [];
  for (const tab of snapshot.tabs) {
    if (
      !isStableId(tab.id) ||
      tab.panes.length < 1 ||
      tab.panes.length > MAX_PANES_PER_TAB ||
      (tab.dual && tab.panes.length !== 2)
    )
      return null;
    const activePaneIndex = tab.panes.findIndex(
      (pane) => pane.id === tab.activePaneId,
    );
    if (activePaneIndex < 0) return null;

    const panes: PersistedWorkbenchPane[] = [];
    for (const pane of tab.panes) {
      if (!isStableId(pane.id) || pane.path.length > MAX_PATH_LENGTH)
        return null;
      const workspace = pane.controller.workspace;
      let restore = workspace.session.serializeRestoreMetadata();
      const currentTarget = workspace.session.currentTarget;
      const currentTargetKey = workbenchTargetKey(currentTarget);
      const pathMatchesTarget = currentTarget !== null && pane.pathTargetKey === currentTargetKey;
      if (
        restore === null &&
        platform !== null &&
        currentTarget?.kind === "browse" &&
        pathMatchesTarget &&
        pane.path.trim().length > 0
      ) {
        restore = serializeWorkspaceRestoreMetadata({
          kind: "browse",
          platform,
          routingHint: pane.path,
          displayHint: pane.path,
        });
      }
      panes.push({
        id: pane.id,
        path: pathMatchesTarget
          ? pane.path
          : restore?.locator.kind === "browse"
            ? restore.locator.routingHint
            : "",
        restore,
      });
    }
    tabs.push({
      id: tab.id,
      panes,
      activePaneIndex,
      dual: tab.dual,
    });
  }
  const parsed = parsePersistedWorkbench({
    version: WORKSPACE_VERSION,
    tabs,
    activeTabIndex,
  });
  return parsed;
}

function validTreePaneId(paneId: string) {
  return isStableId(paneId);
}

function treeStorageKey(paneId: string) {
  return `${TREE_STORAGE_PREFIX}${paneId}`;
}

/** Reads a small, non-authoritative set of expanded tree paths for one pane. */
export function readExpandedTreePaths(paneId: string): string[] {
  if (!validTreePaneId(paneId)) return [];
  const storage = localStorageOrNull();
  if (!storage) return [];
  try {
    const raw = storage.getItem(treeStorageKey(paneId));
    if (!raw || raw.length > 128 * 1024) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const paths = [...new Set(parsed.filter(
      (path): path is string =>
        typeof path === "string" && path.length > 0 && path.length <= MAX_PATH_LENGTH,
    ))].slice(0, MAX_PERSISTED_TREE_PATHS);
    return paths;
  } catch {
    return [];
  }
}

/** Stores display/routing paths only; never Browse refs or write authority. */
export function writeExpandedTreePaths(paneId: string, paths: string[]) {
  if (!validTreePaneId(paneId)) return false;
  const storage = localStorageOrNull();
  if (!storage) return false;
  const bounded = [...new Set(paths.filter(
    (path) => typeof path === "string" && path.length > 0 && path.length <= MAX_PATH_LENGTH,
  ))].slice(0, MAX_PERSISTED_TREE_PATHS);
  try {
    const serialized = JSON.stringify(bounded);
    if (serialized.length > 128 * 1024) return false;
    storage.setItem(treeStorageKey(paneId), serialized);
    return true;
  } catch {
    return false;
  }
}

/** Removes only one pane's bounded, display-only tree expansion preference. */
export function clearExpandedTreePaths(paneId: string) {
  if (!validTreePaneId(paneId)) return false;
  const storage = localStorageOrNull();
  if (!storage) return false;
  try {
    storage.removeItem(treeStorageKey(paneId));
    return true;
  } catch {
    return false;
  }
}
