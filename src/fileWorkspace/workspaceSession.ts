import type {
  BrowseQuerySpecV1,
  BrowsePathRef,
  LibraryNavigationSource,
  LocationRef,
  NavigationTarget,
  WorkspaceRestoreLocator
} from "../types/fileWorkspace";

export const WORKSPACE_RESTORE_SCHEMA_VERSION = 1 as const;

export type WorkspaceMode = NavigationTarget["kind"];
export type LibraryNavigationTarget = Extract<NavigationTarget, { kind: "library" }>;
export type BrowseNavigationTarget = Extract<NavigationTarget, { kind: "browse" }>;
export type WorkspaceViewMode = "list" | "grid";

/**
 * Bounded presentation state is session/UI context only. It is deliberately
 * separate from NavigationTarget so it cannot become a file or location
 * authority when restore metadata is serialized.
 */
export interface WorkspacePresentationState {
  viewMode?: WorkspaceViewMode;
  scrollAnchor?: string;
  contextOpen?: boolean;
  /** Browse-local query text/kind restored with the history-owned target. */
  browseQuery?: BrowseQuerySpecV1;
}

export interface WorkspaceRestoreMetadata {
  version: typeof WORKSPACE_RESTORE_SCHEMA_VERSION;
  locator: WorkspaceRestoreLocator;
  presentation?: WorkspacePresentationState;
}

export interface WorkspaceSessionOptions {
  initialTarget?: NavigationTarget;
  initialRestoreLocator?: WorkspaceRestoreLocator;
  presentation?: WorkspacePresentationState;
}

export interface WorkspaceNavigationOptions {
  /** Required only when a Browse target should retain non-authoritative restore routing metadata. */
  restoreLocator?: WorkspaceRestoreLocator;
  presentation?: WorkspacePresentationState;
}

export interface WorkspaceSessionSnapshot {
  currentTarget: NavigationTarget | null;
  history: readonly NavigationTarget[];
  historyIndex: number;
  lastLibraryTarget: LibraryNavigationTarget | null;
  lastBrowseTarget: BrowseNavigationTarget | null;
  requestEpoch: number;
  disposed: boolean;
  presentation: Readonly<WorkspacePresentationState>;
}

/** A non-serializable, session-bound publication right. */
export interface WorkspaceRequestToken {
  readonly epoch: number;
  readonly sessionId: symbol;
}

export type WorkspaceRestoreFailureReason =
  | "invalid_shape"
  | "unsupported_version"
  | "invalid_locator"
  | "invalid_presentation";

export type WorkspaceRestoreParseResult =
  | { ok: true; metadata: WorkspaceRestoreMetadata }
  | { ok: false; reason: WorkspaceRestoreFailureReason };

const LIBRARY_NAVIGATION_SOURCES: readonly LibraryNavigationSource[] = [
  "smart_view",
  "saved_view",
  "tag",
  "search",
  "custom"
];

const WORKSPACE_PLATFORMS = ["macos", "windows"] as const;
const WORKSPACE_VIEW_MODES = ["list", "grid"] as const;
const MAX_RESTORE_TEXT_LENGTH = 4096;
const MAX_SCROLL_ANCHOR_LENGTH = 512;

interface HistoryEntry {
  target: NavigationTarget;
  restoreLocator?: WorkspaceRestoreLocator;
  presentation: WorkspacePresentationState;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(value: Record<string, unknown>, allowedKeys: readonly string[]) {
  const allowed = new Set(allowedKeys);
  return Object.keys(value).every((key) => allowed.has(key));
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function isBoundedNonEmptyString(value: unknown, maxLength: number): value is string {
  return isNonEmptyString(value) && value.length <= maxLength;
}

function isLibraryNavigationSource(value: unknown): value is LibraryNavigationSource {
  return typeof value === "string"
    && LIBRARY_NAVIGATION_SOURCES.includes(value as LibraryNavigationSource);
}

function isLocationRef(value: unknown): value is LocationRef {
  if (!isRecord(value) || typeof value.kind !== "string") return false;

  if (value.kind === "managed") {
    return hasOnlyKeys(value, ["kind", "scanRootId"])
      && isNonEmptyString(value.scanRootId);
  }

  if (value.kind === "ephemeral") {
    return hasOnlyKeys(value, ["kind", "browseSessionId", "locationId"])
      && isNonEmptyString(value.browseSessionId)
      && isNonEmptyString(value.locationId);
  }

  return false;
}

function isBrowsePathRef(value: unknown): value is BrowsePathRef {
  return isRecord(value)
    && hasOnlyKeys(value, ["id"])
    && isNonEmptyString(value.id);
}

function isBrowseQuerySpec(value: unknown): value is BrowseQuerySpecV1 {
  if (!isRecord(value) || !hasOnlyKeys(value, ["text", "entryKind"])) return false;
  return (value.text === null || isBoundedNonEmptyString(value.text, MAX_RESTORE_TEXT_LENGTH))
    && (value.entryKind === "all" || value.entryKind === "file" || value.entryKind === "directory");
}

/** Runtime validation keeps untrusted callers from smuggling paths or extra authority fields in. */
export function isNavigationTarget(value: unknown): value is NavigationTarget {
  if (!isRecord(value) || typeof value.kind !== "string") return false;

  if (value.kind === "library") {
    return hasOnlyKeys(value, ["kind", "source", "key"])
      && isLibraryNavigationSource(value.source)
      && isNonEmptyString(value.key);
  }

  return value.kind === "browse"
    && hasOnlyKeys(value, ["kind", "location", "pathRef"])
    && isLocationRef(value.location)
    && isBrowsePathRef(value.pathRef);
}

/** Runtime validation for the W1-01 non-authoritative restore contract. */
export function isWorkspaceRestoreLocator(value: unknown): value is WorkspaceRestoreLocator {
  if (!isRecord(value) || typeof value.kind !== "string") return false;

  if (value.kind === "library") {
    return hasOnlyKeys(value, ["kind", "source", "key"])
      && isLibraryNavigationSource(value.source)
      && isNonEmptyString(value.key);
  }

  return value.kind === "browse"
    && hasOnlyKeys(value, ["kind", "platform", "routingHint", "displayHint"])
    && typeof value.platform === "string"
    && WORKSPACE_PLATFORMS.includes(value.platform as (typeof WORKSPACE_PLATFORMS)[number])
    && isBoundedNonEmptyString(value.routingHint, MAX_RESTORE_TEXT_LENGTH)
    && (value.displayHint === undefined
      || isBoundedNonEmptyString(value.displayHint, MAX_RESTORE_TEXT_LENGTH));
}

export function isWorkspacePresentationState(value: unknown): value is WorkspacePresentationState {
  if (!isRecord(value) || !hasOnlyKeys(value, ["viewMode", "scrollAnchor", "contextOpen", "browseQuery"])) return false;
  return (value.viewMode === undefined
    || (typeof value.viewMode === "string"
      && WORKSPACE_VIEW_MODES.includes(value.viewMode as WorkspaceViewMode)))
    && (value.scrollAnchor === undefined
      || isBoundedNonEmptyString(value.scrollAnchor, MAX_SCROLL_ANCHOR_LENGTH))
    && (value.contextOpen === undefined || typeof value.contextOpen === "boolean")
    && (value.browseQuery === undefined || isBrowseQuerySpec(value.browseQuery));
}

function cloneLocation(location: LocationRef): LocationRef {
  return location.kind === "managed"
    ? { kind: "managed", scanRootId: location.scanRootId }
    : {
        kind: "ephemeral",
        browseSessionId: location.browseSessionId,
        locationId: location.locationId
      };
}

function cloneTarget(target: NavigationTarget): NavigationTarget {
  return target.kind === "library"
    ? { kind: "library", source: target.source, key: target.key }
    : {
        kind: "browse",
        location: cloneLocation(target.location),
        pathRef: { id: target.pathRef.id }
      };
}

function cloneRestoreLocator(locator: WorkspaceRestoreLocator): WorkspaceRestoreLocator {
  return locator.kind === "library"
    ? { kind: "library", source: locator.source, key: locator.key }
    : {
        kind: "browse",
        platform: locator.platform,
        routingHint: locator.routingHint,
        ...(locator.displayHint === undefined ? {} : { displayHint: locator.displayHint })
      };
}

function clonePresentation(presentation: WorkspacePresentationState): WorkspacePresentationState {
  return {
    ...(presentation.viewMode === undefined ? {} : { viewMode: presentation.viewMode }),
    ...(presentation.scrollAnchor === undefined ? {} : { scrollAnchor: presentation.scrollAnchor }),
    ...(presentation.contextOpen === undefined ? {} : { contextOpen: presentation.contextOpen }),
    ...(presentation.browseQuery === undefined ? {} : {
      browseQuery: {
        text: presentation.browseQuery.text,
        entryKind: presentation.browseQuery.entryKind
      }
    })
  };
}

function targetsEqual(left: NavigationTarget, right: NavigationTarget) {
  if (left.kind !== right.kind) return false;
  if (left.kind === "library" && right.kind === "library") {
    return left.source === right.source && left.key === right.key;
  }
  if (left.kind !== "browse" || right.kind !== "browse") return false;
  const locationsEqual = left.location.kind === right.location.kind
    && (left.location.kind === "managed"
      ? right.location.kind === "managed"
        && left.location.scanRootId === right.location.scanRootId
      : right.location.kind === "ephemeral"
        && left.location.browseSessionId === right.location.browseSessionId
        && left.location.locationId === right.location.locationId);
  return locationsEqual && left.pathRef.id === right.pathRef.id;
}

function restoreLocatorsEqual(
  left: WorkspaceRestoreLocator | undefined,
  right: WorkspaceRestoreLocator | undefined
) {
  if (left === undefined || right === undefined) return left === right;
  if (left.kind !== right.kind) return false;
  if (left.kind === "library" && right.kind === "library") {
    return left.source === right.source && left.key === right.key;
  }
  if (left.kind !== "browse" || right.kind !== "browse") return false;
  return left.platform === right.platform
    && left.routingHint === right.routingHint
    && left.displayHint === right.displayHint;
}

function libraryRestoreLocatorForTarget(target: LibraryNavigationTarget): WorkspaceRestoreLocator {
  return { kind: "library", source: target.source, key: target.key };
}

function isRestoreLocatorForTarget(
  target: NavigationTarget,
  locator: WorkspaceRestoreLocator
) {
  if (target.kind !== locator.kind) return false;
  return target.kind === "library" && locator.kind === "library"
    ? target.source === locator.source && target.key === locator.key
    : target.kind === "browse" && locator.kind === "browse";
}

function parsePresentation(value: unknown): WorkspacePresentationState | null {
  return isWorkspacePresentationState(value) ? clonePresentation(value) : null;
}

/**
 * Produces a JSON-safe restore projection. It intentionally accepts a locator,
 * not a live NavigationTarget, so ephemeral refs cannot be serialized by this
 * boundary.
 */
export function serializeWorkspaceRestoreMetadata(
  locator: WorkspaceRestoreLocator,
  presentation?: WorkspacePresentationState
): WorkspaceRestoreMetadata | null {
  if (!isWorkspaceRestoreLocator(locator)) return null;
  if (presentation !== undefined && !isWorkspacePresentationState(presentation)) return null;

  const metadata: WorkspaceRestoreMetadata = {
    version: WORKSPACE_RESTORE_SCHEMA_VERSION,
    locator: cloneRestoreLocator(locator)
  };
  if (presentation !== undefined && Object.keys(presentation).length > 0) {
    metadata.presentation = clonePresentation(presentation);
  }
  return metadata;
}

/**
 * Parses only the versioned non-authoritative restore projection. Invalid data
 * is returned to the caller as a failure; no target is guessed or revived.
 */
export function parseWorkspaceRestoreMetadata(value: unknown): WorkspaceRestoreParseResult {
  if (!isRecord(value)
    || !hasOnlyKeys(value, ["version", "locator", "presentation"])) {
    return { ok: false, reason: "invalid_shape" };
  }
  if (value.version !== WORKSPACE_RESTORE_SCHEMA_VERSION) {
    return { ok: false, reason: "unsupported_version" };
  }
  if (!isWorkspaceRestoreLocator(value.locator)) {
    return { ok: false, reason: "invalid_locator" };
  }
  if (value.presentation !== undefined && !isWorkspacePresentationState(value.presentation)) {
    return { ok: false, reason: "invalid_presentation" };
  }

  const metadata: WorkspaceRestoreMetadata = {
    version: WORKSPACE_RESTORE_SCHEMA_VERSION,
    locator: cloneRestoreLocator(value.locator)
  };
  if (value.presentation !== undefined) {
    metadata.presentation = clonePresentation(value.presentation);
  }
  return { ok: true, metadata };
}

/** Safe reconstruction exists only for the stable managed-library key. */
export function navigationTargetFromLibraryRestoreLocator(
  locator: WorkspaceRestoreLocator
): LibraryNavigationTarget | null {
  if (!isWorkspaceRestoreLocator(locator) || locator.kind !== "library") return null;
  return { kind: "library", source: locator.source, key: locator.key };
}

export class WorkspaceSession {
  private readonly sessionId = Symbol("workspace-session");
  private historyEntries: HistoryEntry[] = [];
  private historyIndex = -1;
  private lastLibrary: LibraryNavigationTarget | null = null;
  private lastBrowse: BrowseNavigationTarget | null = null;
  private requestEpochValue = 0;
  private disposedValue = false;
  private pendingPresentation: WorkspacePresentationState = {};

  constructor(options: WorkspaceSessionOptions = {}) {
    const initialPresentation = options.presentation === undefined
      ? {}
      : parsePresentation(options.presentation);
    if (initialPresentation === null) {
      throw new TypeError("WorkspaceSession presentation state is invalid");
    }
    if (options.initialTarget !== undefined) {
      const initialEntry = this.buildEntry(
        options.initialTarget,
        options.initialRestoreLocator,
        initialPresentation
      );
      if (initialEntry === null) {
        throw new TypeError("WorkspaceSession initial navigation target or restore locator is invalid");
      }
      this.historyEntries.push(initialEntry);
      this.historyIndex = 0;
      this.rememberTarget(initialEntry.target);
    } else if (options.initialRestoreLocator !== undefined) {
      throw new TypeError("WorkspaceSession restore locator requires an initial target");
    } else {
      this.pendingPresentation = initialPresentation;
    }
  }

  get currentTarget() {
    const entry = this.currentEntry();
    return entry === undefined ? null : cloneTarget(entry.target);
  }

  get requestEpoch() {
    return this.requestEpochValue;
  }

  get disposed() {
    return this.disposedValue;
  }

  getState(): WorkspaceSessionSnapshot {
    const currentTarget = this.currentTarget;
    return {
      currentTarget,
      history: this.historyEntries.map((entry) => cloneTarget(entry.target)),
      historyIndex: this.historyIndex,
      lastLibraryTarget: this.lastLibrary === null ? null : cloneTarget(this.lastLibrary) as LibraryNavigationTarget,
      lastBrowseTarget: this.lastBrowse === null ? null : cloneTarget(this.lastBrowse) as BrowseNavigationTarget,
      requestEpoch: this.requestEpochValue,
      disposed: this.disposedValue,
      presentation: clonePresentation(this.currentPresentation())
    };
  }

  getSnapshot() {
    return this.getState();
  }

  navigate(target: NavigationTarget, options: WorkspaceNavigationOptions = {}) {
    if (this.disposedValue) return false;

    const presentation = options.presentation === undefined
      ? this.currentEntry() === undefined ? this.pendingPresentation : {}
      : parsePresentation(options.presentation);
    if (presentation === null) return false;

    const entry = this.buildEntry(target, options.restoreLocator, presentation);
    if (entry === null) return false;

    const currentEntry = this.currentEntry();
    if (currentEntry !== undefined && targetsEqual(currentEntry.target, entry.target)) {
      if (entry.restoreLocator !== undefined
        && !restoreLocatorsEqual(currentEntry.restoreLocator, entry.restoreLocator)) {
        currentEntry.restoreLocator = cloneRestoreLocator(entry.restoreLocator);
      }
      if (options.presentation !== undefined) currentEntry.presentation = presentation;
      return true;
    }

    this.historyEntries = this.historyEntries.slice(0, this.historyIndex + 1);
    this.historyEntries.push(entry);
    this.historyIndex = this.historyEntries.length - 1;
    this.rebuildLastTargetsFromHistory();
    this.rememberTarget(entry.target);
    this.advanceRequestEpoch();
    return true;
  }

  /**
   * Rebind the current history slot to fresh session-scoped refs while
   * retaining the slot's restore metadata and presentation. This is used when
   * Back/Forward returns to a Browse locator: the locator is re-resolved, but
   * its old ephemeral refs are never revived.
   */
  replaceCurrentTarget(target: NavigationTarget, options: WorkspaceNavigationOptions = {}) {
    if (this.disposedValue || this.currentEntry() === undefined) return false;
    const currentEntry = this.currentEntry()!;
    const presentation = options.presentation === undefined
      ? currentEntry.presentation
      : parsePresentation(options.presentation);
    if (presentation === null) return false;
    const entry = this.buildEntry(target, options.restoreLocator, presentation);
    if (entry === null) return false;
    this.historyEntries[this.historyIndex] = entry;
    this.rebuildLastTargetsFromHistory();
    this.rememberTarget(entry.target);
    this.advanceRequestEpoch();
    return true;
  }

  back() {
    if (this.disposedValue || this.historyIndex <= 0) return false;
    this.historyIndex -= 1;
    this.rememberTarget(this.historyEntries[this.historyIndex].target);
    this.advanceRequestEpoch();
    return true;
  }

  goBack() {
    return this.back();
  }

  forward() {
    if (this.disposedValue || this.historyIndex < 0
      || this.historyIndex >= this.historyEntries.length - 1) return false;
    this.historyIndex += 1;
    this.rememberTarget(this.historyEntries[this.historyIndex].target);
    this.advanceRequestEpoch();
    return true;
  }

  goForward() {
    return this.forward();
  }

  /**
   * Direct mode switching navigates to the remembered target as a new
   * chronological history step while retaining that entry's presentation and
   * restore metadata.
   */
  switchMode(mode: WorkspaceMode) {
    if (this.disposedValue || (mode !== "library" && mode !== "browse")) return false;
    const current = this.currentEntry();
    if (current !== undefined && current.target.kind === mode) return true;

    const target = mode === "library" ? this.lastLibrary : this.lastBrowse;
    if (target === null) return false;
    const targetEntry = this.findHistoryEntry(target);
    if (targetEntry === undefined) return false;

    return this.navigate(targetEntry.target, {
      ...(targetEntry.restoreLocator === undefined
        ? {}
        : { restoreLocator: targetEntry.restoreLocator }),
      presentation: targetEntry.presentation
    });
  }

  switchToLibrary() {
    return this.switchMode("library");
  }

  switchToBrowse() {
    return this.switchMode("browse");
  }

  beginRequest(): WorkspaceRequestToken {
    return Object.freeze({ epoch: this.requestEpochValue, sessionId: this.sessionId });
  }

  canPublish(token: WorkspaceRequestToken) {
    return !this.disposedValue
      && token !== null
      && typeof token === "object"
      && token.sessionId === this.sessionId
      && token.epoch === this.requestEpochValue;
  }

  isRequestCurrent(token: WorkspaceRequestToken) {
    return this.canPublish(token);
  }

  isEpochCurrent(epoch: number) {
    return !this.disposedValue && Number.isSafeInteger(epoch) && epoch === this.requestEpochValue;
  }

  setPresentation(presentation: WorkspacePresentationState) {
    if (this.disposedValue) return false;
    const parsed = parsePresentation(presentation);
    if (parsed === null) return false;
    const currentEntry = this.currentEntry();
    if (currentEntry === undefined) {
      this.pendingPresentation = parsed;
    } else {
      currentEntry.presentation = parsed;
    }
    return true;
  }

  /**
   * Revokes current request publication rights without changing navigation,
   * history-owned refs or live presentation state. Frontend lifecycle owners
   * use this when disposable work is suspended temporarily.
   */
  invalidateRequests() {
    if (this.disposedValue) return false;
    this.advanceRequestEpoch();
    return true;
  }

  /** Dispose is idempotent and revokes every token issued before it. */
  dispose() {
    if (this.disposedValue) return false;
    this.disposedValue = true;
    this.advanceRequestEpoch();
    return true;
  }

  /** Returns null when no safe non-authoritative locator exists or the session is disposed. */
  serializeRestoreMetadata() {
    if (this.disposedValue) return null;
    const entry = this.currentEntry();
    if (entry?.restoreLocator === undefined) return null;
    return serializeWorkspaceRestoreMetadata(entry.restoreLocator, this.currentPresentation());
  }

  serializeRestoreLocator() {
    const metadata = this.serializeRestoreMetadata();
    return metadata === null ? null : cloneRestoreLocator(metadata.locator);
  }

  /** Only fully re-admissible history may release its opaque process-local refs. */
  canReadmitBrowseSession(sessionId: string) {
    const entries = this.historyEntries.filter((entry) => entry.target.kind === "browse"
      && entry.target.location.kind === "ephemeral" && entry.target.location.browseSessionId === sessionId);
    return entries.length > 0 && entries.every((entry) => entry.restoreLocator?.kind === "browse");
  }

  private currentEntry() {
    return this.historyIndex < 0 ? undefined : this.historyEntries[this.historyIndex];
  }

  private buildEntry(
    target: NavigationTarget,
    restoreLocator: WorkspaceRestoreLocator | undefined,
    presentation: WorkspacePresentationState
  ): HistoryEntry | null {
    if (!isNavigationTarget(target)) return null;
    if (restoreLocator !== undefined && !isWorkspaceRestoreLocator(restoreLocator)) return null;
    if (restoreLocator !== undefined && !isRestoreLocatorForTarget(target, restoreLocator)) return null;

    const clonedTarget = cloneTarget(target);
    return {
      target: clonedTarget,
      restoreLocator: target.kind === "library"
        ? libraryRestoreLocatorForTarget(target)
        : restoreLocator === undefined ? undefined : cloneRestoreLocator(restoreLocator),
      presentation: clonePresentation(presentation)
    };
  }

  private rememberTarget(target: NavigationTarget) {
    if (target.kind === "library") {
      this.lastLibrary = cloneTarget(target) as LibraryNavigationTarget;
    } else {
      this.lastBrowse = cloneTarget(target) as BrowseNavigationTarget;
    }
  }

  private rebuildLastTargetsFromHistory() {
    this.lastLibrary = null;
    this.lastBrowse = null;
    for (const entry of this.historyEntries) this.rememberTarget(entry.target);
  }

  private findHistoryEntry(target: NavigationTarget) {
    for (let index = this.historyEntries.length - 1; index >= 0; index -= 1) {
      if (targetsEqual(this.historyEntries[index].target, target)) return this.historyEntries[index];
    }
    return undefined;
  }

  private currentPresentation() {
    const entry = this.currentEntry();
    return entry === undefined ? this.pendingPresentation : entry.presentation;
  }

  private advanceRequestEpoch() {
    if (this.requestEpochValue >= Number.MAX_SAFE_INTEGER) {
      throw new Error("WorkspaceSession request epoch exhausted");
    }
    this.requestEpochValue += 1;
  }
}

export function createWorkspaceSession(options: WorkspaceSessionOptions = {}) {
  return new WorkspaceSession(options);
}
