import { isBrowserMockEnabled } from "../../utils/runtimeMode";
import type {
  BrowseOpenRequest,
  BrowseOpenResponse,
  BrowseQuerySpecV1,
  LocationRef,
  NavigationTarget,
  WorkspaceRestoreLocator
} from "../../types/fileWorkspace";
import {
  FileWorkspaceController,
  WorkspaceSession,
  type FileWorkspaceControllerState
} from "../../fileWorkspace";
import type { WorkspaceViewMode } from "../../fileWorkspace";

/** Explicit compatibility fixture; never available inside the Tauri runtime. */
export function isManagedLibraryBrowserFixture() {
  return isBrowserMockEnabled()
    && new URLSearchParams(globalThis.location?.search ?? "").get("managed-library-test") === "1";
}

export type FileLibraryMode = "library" | "browse";

/**
 * Internal W2 migration target only. The user-facing shell always localizes
 * Library from the product label and must never display this key.
 *
 * ADR-0016 keeps the Managed Library implementation, Query V2 authority,
 * LibrarySelectionV1, tags and Saved Views, but the ordinary user-visible
 * Files workspace starts in Browse. This target therefore stays available for
 * explicit compatibility callers and tests; it is no longer the default
 * workspace entry.
 */
export const LEGACY_LIBRARY_MIGRATION_TARGET: NavigationTarget = {
  kind: "library",
  source: "custom",
  key: "legacy_library"
};

export interface FileLibraryExperienceState {
  mode: FileLibraryMode;
  detachedBrowse: boolean;
  workspace: FileWorkspaceControllerState;
}

export class FileLibraryExperienceController {
  readonly workspace: FileWorkspaceController;

  private readonly listeners = new Set<(state: FileLibraryExperienceState) => void>();
  private readonly unsubscribeWorkspace: () => boolean;
  private modeValue: FileLibraryMode = "browse";
  private detachedBrowseValue = true;
  private disposedValue = false;
  private authoritativeTargetSignature: string;
  private stateValue: FileLibraryExperienceState;

  constructor(workspace = createDefaultWorkspaceController()) {
    this.workspace = workspace;
    if (isManagedLibraryBrowserFixture()) {
      workspace.navigate(LEGACY_LIBRARY_MIGRATION_TARGET);
      this.modeValue = "library";
      this.detachedBrowseValue = false;
    }
    this.authoritativeTargetSignature = navigationTargetSignature(workspace.getState().session.currentTarget);
    this.stateValue = this.composeState(workspace.getState());
    this.unsubscribeWorkspace = workspace.subscribe((state) => this.syncFromWorkspace(state));
  }

  getState() {
    return this.stateValue;
  }

  subscribe(listener: (state: FileLibraryExperienceState) => void) {
    this.listeners.add(listener);
    listener(this.stateValue);
    return () => this.listeners.delete(listener);
  }

  navigate(target: NavigationTarget, options?: Parameters<FileWorkspaceController["navigate"]>[1]) {
    if (this.disposedValue) return false;
    const changed = this.workspace.navigate(target, options);
    this.syncFromWorkspace();
    return changed;
  }

  async openBrowse(request: BrowseOpenRequest, accept?: () => boolean | Promise<boolean>): Promise<BrowseOpenResponse | null> {
    if (this.disposedValue) return null;
    const response = await this.workspace.openBrowse(request, accept);
    if (response !== null) {
      this.modeValue = "browse";
      this.detachedBrowseValue = false;
    }
    this.syncFromWorkspace();
    return response;
  }

  async restoreBrowse(locator: WorkspaceRestoreLocator): Promise<BrowseOpenResponse | null> {
    if (this.disposedValue) return null;
    const response = await this.workspace.restoreBrowse(locator);
    if (response !== null) {
      this.modeValue = "browse";
      this.detachedBrowseValue = false;
    }
    this.syncFromWorkspace();
    return response;
  }

  /**
   * Re-admits one backend-issued LocationRef through the workspace action
   * seam. This keeps future Location navigation out of renderer path and
   * restore-locator reconstruction while preserving the existing lifecycle
   * owner in FileWorkspaceController.
   */
  async browseLocation(location: LocationRef): Promise<BrowseOpenResponse | null> {
    if (this.disposedValue) return null;
    const response = await this.workspace.browseLocation(location);
    if (response !== null) {
      this.modeValue = "browse";
      this.detachedBrowseValue = false;
    }
    this.syncFromWorkspace();
    return response;
  }

  /**
   * First-entry Browse is intentionally only a projection change. Once W1 has
   * a remembered Browse target, the controller owns the real mode transition
   * and its disposable-work cleanup chronology.
   */
  async switchMode(mode: FileLibraryMode) {
    if (this.disposedValue || this.workspace.getState().suspended) return false;

    const workspaceState = this.workspace.getState();
    const currentTarget = workspaceState.session.currentTarget;
    if (currentTarget?.kind === mode && !this.detachedBrowseValue) {
      this.modeValue = mode;
      this.syncFromWorkspace(workspaceState);
      return true;
    }

    if (mode === "browse" && workspaceState.session.lastBrowseTarget === null) {
      this.modeValue = "browse";
      this.detachedBrowseValue = true;
      this.publishProjection(workspaceState);
      return true;
    }

    if (mode === "library" && this.detachedBrowseValue) {
      this.modeValue = "library";
      this.detachedBrowseValue = false;
      this.publishProjection(workspaceState);
      return true;
    }

    const switched = await this.workspace.switchMode(mode);
    if (!switched) return false;
    this.modeValue = mode;
    this.detachedBrowseValue = false;
    this.syncFromWorkspace();
    return true;
  }

  /**
   * Presentation-only chrome. WorkspaceSession keeps the view mode on the
   * current target's history entry; changing it must not create navigation.
   */
  setViewMode(viewMode: WorkspaceViewMode) {
    if (this.disposedValue || (viewMode !== "list" && viewMode !== "grid")) return false;
    const presentation = this.workspace.getState().session.presentation;
    const changed = this.workspace.session.setPresentation({ ...presentation, viewMode });
    this.syncFromWorkspace();
    return changed;
  }

  /**
   * Presentation-only Context preference. WorkspaceSession keeps it on the
   * current target entry without creating a navigation step or a new store.
   */
  setContextOpen(contextOpen: boolean) {
    if (this.disposedValue || typeof contextOpen !== "boolean") return false;
    const presentation = this.workspace.getState().session.presentation;
    const changed = this.workspace.session.setPresentation({ ...presentation, contextOpen });
    this.syncFromWorkspace();
    return changed;
  }

  /**
   * Browse query text/kind is presentation state on the current
   * WorkspaceSession history entry. It restores with Back/Forward without
   * creating a second query authority or a navigation step.
   */
  setBrowseQuery(browseQuery: BrowseQuerySpecV1) {
    if (this.disposedValue) return false;
    const presentation = this.workspace.getState().session.presentation;
    const changed = this.workspace.session.setPresentation({ ...presentation, browseQuery });
    this.syncFromWorkspace();
    return changed;
  }

  async back() {
    if (this.disposedValue || this.workspace.getState().suspended) return false;
    const before = this.workspace.getState().session.historyIndex;
    await this.workspace.back();
    const after = this.workspace.getState().session.historyIndex;
    this.syncFromWorkspace();
    return before !== after;
  }

  async forward() {
    if (this.disposedValue || this.workspace.getState().suspended) return false;
    const before = this.workspace.getState().session.historyIndex;
    await this.workspace.forward();
    const after = this.workspace.getState().session.historyIndex;
    this.syncFromWorkspace();
    return before !== after;
  }

  async suspend() {
    if (this.disposedValue) return false;
    const suspended = await this.workspace.suspend();
    this.syncFromWorkspace();
    return suspended;
  }

  async resume() {
    if (this.disposedValue) return false;
    const resumed = await this.workspace.resume();
    this.syncFromWorkspace();
    return resumed;
  }

  async dispose() {
    if (this.disposedValue) return false;
    this.disposedValue = true;
    this.unsubscribeWorkspace();
    await this.workspace.dispose();
    this.syncFromWorkspace();
    return true;
  }

  private composeState(workspace: FileWorkspaceControllerState): FileLibraryExperienceState {
    return {
      mode: this.modeValue,
      detachedBrowse: this.detachedBrowseValue,
      workspace
    };
  }

  private syncFromWorkspace(workspace = this.workspace.getState()) {
    const currentTarget = workspace.session.currentTarget;
    const nextTargetSignature = navigationTargetSignature(currentTarget);
    const targetChanged = nextTargetSignature !== this.authoritativeTargetSignature;
    if (targetChanged) {
      this.authoritativeTargetSignature = nextTargetSignature;
      if (currentTarget !== null) {
        this.modeValue = currentTarget.kind;
        this.detachedBrowseValue = false;
      }
    }
    this.stateValue = this.composeState(workspace);
    this.emit();
  }

  private publishProjection(workspace = this.workspace.getState()) {
    this.stateValue = this.composeState(workspace);
    this.emit();
  }

  private emit() {
    for (const listener of this.listeners) listener(this.stateValue);
  }
}

function createDefaultWorkspaceController() {
  // ADR-0016: the ordinary Files workspace opens in Browse with no folder
  // open. No Managed Library target is fabricated on first entry, so nothing
  // is adopted, scanned or authorized merely by visiting Files.
  return new FileWorkspaceController(undefined, new WorkspaceSession());
}

function navigationTargetSignature(target: NavigationTarget | null) {
  if (target === null) return "none";
  if (target.kind === "library") return `library:${target.source}:${target.key}`;
  const location = target.location.kind === "managed"
    ? `managed:${target.location.scanRootId}`
    : `ephemeral:${target.location.browseSessionId}:${target.location.locationId}`;
  return `browse:${location}:${target.pathRef.id}`;
}
