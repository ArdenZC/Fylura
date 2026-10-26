import type {
  BrowseEnumerationRef,
  BrowseNextPageRequest,
  BrowseOpenRequest,
  BrowseOpenResponse,
  BrowsePage,
  BrowsePathRef,
  BrowseQuerySpecV1,
  BrowseRestoreRequest,
  BrowseStartEnumerationRequest,
  ChangePendingResponse,
  ChangeStartResponse,
  LocationRef,
  LocationDescriptor,
  NavigationTarget,
  PreviewCreateRequest,
  PreviewAssetArtifact,
  PreviewAssetRequest,
  PreviewNativePresentation,
  PreviewHostKind,
  PreviewSourceRef,
  PreviewSnapshot,
  PreviewSwitchSourceRequest,
  ReadEligibilityResponse,
  ThumbnailArtifact,
  ThumbnailRequest,
  WorkspaceRestoreLocator
} from "../types/fileWorkspace";
import { fileWorkspaceApi, type FileWorkspaceApi } from "../api/fileWorkspaceApi";
import {
  WorkspaceSession,
  type WorkspaceNavigationOptions,
  type WorkspaceRequestToken,
  type WorkspaceSessionSnapshot
} from "./workspaceSession";

export interface FileWorkspaceControllerState {
  session: WorkspaceSessionSnapshot;
  suspended: boolean;
  browse: BrowseOpenResponse | null;
  page: BrowsePage | null;
  change: ChangeStartResponse | null;
  pendingChange: ChangePendingResponse | null;
  locations: LocationDescriptor[];
  lastEligibility: ReadEligibilityResponse | null;
  previews: Record<string, PreviewSnapshot>;
}

export type FileWorkspaceStateListener = (state: FileWorkspaceControllerState) => void;

interface OwnedBrowseSession {
  response: BrowseOpenResponse;
  restoreLocator?: WorkspaceRestoreLocator;
  pages: Map<string, BrowsePage>;
  pathRefs: Map<string, BrowsePathRef>;
  historyPathRefs: Map<string, BrowsePathRef>;
  promotedPathRefs: Set<string>;
  pendingPathRetention: Map<string, Promise<boolean>>;
  unavailable: boolean;
  enumerations: Map<string, BrowseEnumerationRef>;
}

interface PendingEnumeration {
  sessionId: string;
  requestId: string;
  enumeration?: BrowseEnumerationRef;
}

interface PreviewPublication {
  readonly generation: number;
  previewId?: string;
  requestId: string;
  source: PreviewSourceRef;
  hostKind: PreviewHostKind;
  sourceVersion?: string;
}

type PreviewPublicationDraft = Omit<PreviewPublication, "generation">;

interface PreviewSwitchOperation {
  request: PreviewSwitchSourceRequest;
  publication: PreviewPublication;
  token: WorkspaceRequestToken;
  resolve: (snapshot: PreviewSnapshot | null) => void;
  reject: (error: unknown) => void;
}

interface PreviewSwitchQueue {
  inFlight: PreviewSwitchOperation | null;
  pending: PreviewSwitchOperation | null;
  settledPublication?: PreviewPublication;
}

/**
 * Headless W1 coordinator. WorkspaceSession owns publication epochs and the
 * process-local mixed navigation history. This controller owns live backend
 * lifecycle handles, keeping session/path history ownership separate from
 * disposable pages, enumerations, monitors, thumbnails and Preview work.
 */
export class FileWorkspaceController {
  readonly session: WorkspaceSession;
  private readonly api: FileWorkspaceApi;
  private listeners = new Set<FileWorkspaceStateListener>();
  private browseResponse: BrowseOpenResponse | null = null;
  private currentPage: BrowsePage | null = null;
  private changeResponse: ChangeStartResponse | null = null;
  private pendingChangeValue: ChangePendingResponse | null = null;
  private locationsValue: LocationDescriptor[] = [];
  private eligibilityValue: ReadEligibilityResponse | null = null;
  private previewsValue = new Map<string, PreviewSnapshot>();

  // These registries are controller ownership, not new product authorities.
  // WorkspaceSession history retains restore locators; these maps retain only
  // live command-addressable disposable work.
  private ownedBrowseSessions = new Map<string, OwnedBrowseSession>();
  private ownedMonitors = new Map<string, string>();
  private pendingEnumerations = new Map<string, PendingEnumeration>();
  private activeThumbnailRequests = new Set<string>();
  private ownedPreviewIds = new Set<string>();
  // Disposal entries are retained only while backend cleanup is in flight.
  private previewDisposals = new Map<string, Promise<boolean>>();
  /**
   * Request-scoped publication guard layered under WorkspaceSession epochs.
   * One Preview host can receive several overlapping create/switch/start
   * requests; only the latest internal host generation may update previewsValue.
   */
  private previewPublications = new Map<string, PreviewPublication>();
  /** One latest logical publication per Preview host authority. */
  private latestPreviewPublications = new Map<PreviewHostKind, PreviewPublication>();
  /** Monotonic logical publication ordering, independent of backend identity. */
  private previewPublicationGenerations = new Map<PreviewHostKind, number>();
  /**
   * Transport ordering only: PreviewSession remains the backend lifecycle and
   * source authority, while this queue prevents overlapping switch mutations
   * for one live preview session. The pending slot is latest-wins.
   */
  private previewSwitchQueues = new Map<string, PreviewSwitchQueue>();
  private pendingCleanup = new Set<Promise<unknown>>();
  private pageKeys = new WeakMap<BrowsePage, string>();
  private nextPageKey = 0;
  private suspendedValue = false;
  private suspensionPromise: Promise<void> | null = null;

  constructor(api: FileWorkspaceApi = fileWorkspaceApi, session = new WorkspaceSession()) {
    this.api = api;
    this.session = session;
  }

  getState(): FileWorkspaceControllerState {
    return {
      session: this.session.getState(),
      suspended: this.suspendedValue,
      browse: this.browseResponse,
      page: this.currentPage,
      change: this.changeResponse,
      pendingChange: this.pendingChangeValue,
      locations: [...this.locationsValue],
      lastEligibility: this.eligibilityValue,
      previews: Object.fromEntries(this.previewsValue.entries())
    };
  }

  subscribe(listener: FileWorkspaceStateListener) {
    this.listeners.add(listener);
    listener(this.getState());
    return () => this.listeners.delete(listener);
  }

  async openBrowse(request: BrowseOpenRequest, accept: () => boolean | Promise<boolean> = () => true): Promise<BrowseOpenResponse | null> {
    const searchAdmission = request.globalEntryId !== undefined;
    if ((this.suspendedValue && !searchAdmission) || this.session.disposed) return null;
    const token = this.session.beginRequest();
    if (searchAdmission && this.suspensionPromise) await this.suspensionPromise;
    if ((searchAdmission && !(await accept())) || !this.session.canPublish(token)) return null;
    const response = await this.api.browseOpen(request);
    let accepted: boolean;
    try {
      accepted = await accept();
    } catch (error) {
      this.trackCleanup(this.cleanupCall(() => this.api.browseDispose({ sessionId: response.sessionId })));
      throw error;
    }
    if (!accepted || !this.session.canPublish(token)) {
      this.trackCleanup(this.cleanupCall(() => this.api.browseDispose({ sessionId: response.sessionId })));
      return null;
    }
    if (searchAdmission) this.suspendedValue = false;
    return this.publishBrowseAdmission(
      response,
      searchAdmission ? undefined : browseRestoreLocator(request),
      token,
      false
    );
  }

  async restoreBrowse(locator: WorkspaceRestoreLocator): Promise<BrowseOpenResponse | null> {
    if (this.suspendedValue || this.session.disposed || locator.kind !== "browse") return null;
    const token = this.session.beginRequest();
    const request: BrowseRestoreRequest = { locator };
    const response = await this.api.browseRestore(request);
    if (!this.session.canPublish(token)) {
      this.trackCleanup(this.cleanupCall(() => this.api.browseDispose({ sessionId: response.sessionId })));
      return null;
    }
    return this.publishBrowseAdmission(response, locator, token, false);
  }

  /**
   * Re-admits one opaque backend-owned LocationRef. This action has no
   * renderer routing/path input and intentionally receives no restore
   * locator; the returned session is live-process state only.
   */
  async browseLocation(location: LocationRef): Promise<BrowseOpenResponse | null> {
    if (this.suspendedValue || this.session.disposed) return null;
    const token = this.session.beginRequest();
    const response = await this.api.locationBrowse({ location });
    if (!this.session.canPublish(token)) {
      this.trackCleanup(this.cleanupCall(() => this.api.browseDispose({ sessionId: response.sessionId })));
      return null;
    }
    return this.publishBrowseAdmission(response, undefined, token, false);
  }

  navigate(target: NavigationTarget, options: WorkspaceNavigationOptions = {}) {
    if (this.suspendedValue || this.session.disposed) return false;
    const previousEpoch = this.session.requestEpoch;
    const changed = this.session.navigate(target, options);
    const switched = this.session.requestEpoch !== previousEpoch;
    if (switched) {
      this.updateBrowseHistoryOwnership();
      const cleanup = this.teardownCurrentTargetWork();
      this.clearPublishedState();
      this.publishLiveBrowseTargetIfAvailable();
      this.emit();
      void cleanup.then(() => this.finalizeBrowseHistoryOwnership());
    } else if (changed) {
      this.emit();
    }
    return changed;
  }

  /**
   * Switches the live workspace target through the W1 lifecycle owner. UI
   * callers must use this seam instead of calling WorkspaceSession directly so
   * current-target Browse/Preview/thumbnail/change work is always torn down.
   */
  async switchMode(mode: "library" | "browse") {
    if (this.suspendedValue || this.session.disposed) return false;
    const previousEpoch = this.session.requestEpoch;
    if (!this.session.switchMode(mode)) return false;
    if (this.session.requestEpoch === previousEpoch) return true;

    const epoch = this.session.requestEpoch;
    this.updateBrowseHistoryOwnership();
    const cleanup = this.teardownCurrentTargetWork();
    this.clearPublishedState();
    this.publishLiveBrowseTargetIfAvailable();
    this.emit();
    await cleanup;
    if (!this.session.isEpochCurrent(epoch)) return false;
    this.finalizeBrowseHistoryOwnership();
    this.publishLiveBrowseTargetIfAvailable();
    this.emit();
    return true;
  }

  async back(): Promise<BrowseOpenResponse | null> {
    if (this.suspendedValue || !this.session.back()) return null;
    const epoch = this.session.requestEpoch;
    this.updateBrowseHistoryOwnership();
    const cleanup = this.teardownCurrentTargetWork();
    this.clearPublishedState();
    this.emit();
    await cleanup;
    if (!this.session.isEpochCurrent(epoch)) return null;
    this.finalizeBrowseHistoryOwnership();
    const live = this.publishLiveBrowseTargetIfAvailable();
    if (live !== null) {
      this.emit();
      return live;
    }
    return this.restoreCurrentHistoryBrowse();
  }

  async forward(): Promise<BrowseOpenResponse | null> {
    if (this.suspendedValue || !this.session.forward()) return null;
    const epoch = this.session.requestEpoch;
    this.updateBrowseHistoryOwnership();
    const cleanup = this.teardownCurrentTargetWork();
    this.clearPublishedState();
    this.emit();
    await cleanup;
    if (!this.session.isEpochCurrent(epoch)) return null;
    this.finalizeBrowseHistoryOwnership();
    const live = this.publishLiveBrowseTargetIfAvailable();
    if (live !== null) {
      this.emit();
      return live;
    }
    return this.restoreCurrentHistoryBrowse();
  }

  async startEnumeration(
    pathRef?: BrowsePathRef,
    requestId = `browse-${Date.now()}`,
    pageSize = 100,
    query: BrowseQuerySpecV1 = { text: null, entryKind: "all" }
  ): Promise<BrowsePage | null> {
    if (this.suspendedValue || this.session.disposed) return null;
    const browseTarget = this.currentBrowseTarget();
    const token = this.session.beginRequest();
    if (browseTarget === null) return null;
    const sessionId = browseTarget.location.kind === "ephemeral"
      ? browseTarget.location.browseSessionId
      : "";
    const owner = this.ownedBrowseSessions.get(sessionId);
    if (owner === undefined) return null;
    const requestedPathRef = pathRef ?? browseTarget.pathRef;
    owner.pathRefs.set(requestedPathRef.id, requestedPathRef);
    const pending: PendingEnumeration = { sessionId, requestId };
    const pendingKey = `start:${sessionId}:${requestId}`;
    this.pendingEnumerations.set(pendingKey, pending);
    try {
      const retention = this.waitForPathRetention(owner, requestedPathRef.id);
      const retained = typeof retention === "boolean" ? retention : await retention;
      if (!retained || !this.session.canPublish(token)) return null;
      const request: BrowseStartEnumerationRequest = {
        sessionId,
        requestId,
        pathRef: requestedPathRef,
        pageSize,
        query
      };
      const page = await this.api.browseStartEnumeration(request);
      if (!this.session.canPublish(token)) {
        await this.releasePageLater(page);
        return null;
      }
      pending.enumeration = enumerationForPage(page);
      owner.promotedPathRefs.add(requestedPathRef.id);
      this.publishPage(page);
      await this.releasePagesForSupersededEnumerations(owner, page.enumerationId);
      if (!this.session.canPublish(token)) return null;
      return page;
    } finally {
      this.pendingEnumerations.delete(pendingKey);
    }
  }

  async nextPage(pageSize = 100): Promise<BrowsePage | null> {
    if (this.suspendedValue || this.session.disposed) return null;
    const browseTarget = this.currentBrowseTarget();
    const currentPage = this.currentPage;
    if (browseTarget === null || currentPage?.nextCursor === undefined) return null;
    const sessionId = browseTarget.location.kind === "ephemeral"
      ? browseTarget.location.browseSessionId
      : "";
    const owner = this.ownedBrowseSessions.get(sessionId);
    if (owner === undefined) return null;
    const enumeration = owner.enumerations.get(currentPage.enumerationId);
    if (enumeration === undefined) return null;
    const token = this.session.beginRequest();
    const pending: PendingEnumeration = {
      sessionId,
      requestId: enumeration.requestId,
      enumeration
    };
    const pendingKey = `next:${sessionId}:${enumeration.enumerationId}:${token.epoch}`;
    this.pendingEnumerations.set(pendingKey, pending);
    const request: BrowseNextPageRequest = {
      sessionId,
      cursor: currentPage.nextCursor,
      pageSize
    };
    try {
      const page = await this.api.browseNextPage(request);
      if (!this.session.canPublish(token)) {
        await this.releasePageLater(page);
        return null;
      }
      this.publishPage(page);
      return page;
    } finally {
      this.pendingEnumerations.delete(pendingKey);
    }
  }

  async startChange(pathRef: BrowsePathRef): Promise<ChangeStartResponse | null> {
    if (this.suspendedValue || this.session.disposed) return null;
    const target = this.currentBrowseTarget();
    if (target === null || target.location.kind !== "ephemeral") return null;
    const token = this.session.beginRequest();
    const response = await this.api.changeStart({
      sessionId: target.location.browseSessionId,
      pathRef
    });
    if (!this.session.canPublish(token)) {
      this.trackCleanup(this.cleanupCall(() => this.api.changeDispose({ monitorId: response.monitorId })));
      return null;
    }
    const owner = this.ownedBrowseSessions.get(response.sessionId);
    if (owner === undefined) {
      this.trackCleanup(this.cleanupCall(() => this.api.changeDispose({ monitorId: response.monitorId })));
      return null;
    }
    owner.pathRefs.set(pathRef.id, pathRef);
    this.ownedMonitors.set(response.monitorId, response.sessionId);
    this.changeResponse = response;
    this.pendingChangeValue = null;
    this.emit();
    return response;
  }

  async readPendingChange(): Promise<ChangePendingResponse | null> {
    if (this.suspendedValue || this.session.disposed || this.changeResponse === null) return null;
    const token = this.session.beginRequest();
    const pending = await this.api.changePending({ monitorId: this.changeResponse.monitorId });
    if (!this.session.canPublish(token)) return null;
    this.pendingChangeValue = pending;
    this.emit();
    return pending;
  }

  async refreshChange(
    requestId = `refresh-${Date.now()}`,
    pageSize = 100,
    query: BrowseQuerySpecV1 = { text: null, entryKind: "all" }
  ): Promise<BrowsePage | null> {
    if (this.suspendedValue || this.session.disposed || this.changeResponse === null) return null;
    const monitorId = this.changeResponse.monitorId;
    const sessionId = this.ownedMonitors.get(monitorId);
    if (sessionId === undefined) return null;
    const token = this.session.beginRequest();
    const pendingKey = `refresh:${sessionId}:${requestId}`;
    this.pendingEnumerations.set(pendingKey, { sessionId, requestId });
    try {
      const page = await this.api.changeRefresh({ monitorId, requestId, pageSize, query });
      if (!this.session.canPublish(token)) {
        await this.releasePageLater(page);
        return null;
      }
      this.publishPage(page);
      const owner = this.ownedBrowseSessions.get(sessionId);
      if (owner !== undefined) {
        await this.releasePagesForSupersededEnumerations(owner, page.enumerationId);
      }
      if (!this.session.canPublish(token)) return null;
      this.pendingChangeValue = null;
      this.emit();
      return page;
    } finally {
      this.pendingEnumerations.delete(pendingKey);
    }
  }

  async loadLocations(): Promise<LocationDescriptor[] | null> {
    if (this.suspendedValue || this.session.disposed) return null;
    const token = this.session.beginRequest();
    const locations = await this.api.locationList();
    if (!this.session.canPublish(token)) return null;
    this.locationsValue = locations;
    this.emit();
    return locations;
  }

  async readEligibility(source: ReadEligibilityResponse["source"]): Promise<ReadEligibilityResponse | null> {
    if (this.suspendedValue || this.session.disposed) return null;
    const token = this.session.beginRequest();
    const response = await this.api.readEligibility({ source });
    if (!this.session.canPublish(token)) return null;
    this.eligibilityValue = response;
    this.emit();
    return response;
  }

  async requestThumbnail(request: ThumbnailRequest): Promise<ThumbnailArtifact | null> {
    if (this.suspendedValue || this.session.disposed) return null;
    const token = this.session.beginRequest();
    this.activeThumbnailRequests.add(request.requestId);
    try {
      const artifact = await this.api.thumbnailRequest(request);
      if (!this.session.canPublish(token)) {
        this.trackCleanup(this.cleanupCall(() => this.api.thumbnailCancel({ requestId: request.requestId })));
        return null;
      }
      return artifact;
    } finally {
      this.activeThumbnailRequests.delete(request.requestId);
    }
  }

  /**
   * Cancels one presentation-owned request through the existing thumbnail
   * seam. Target teardown remains the owner of bulk cancellation.
   */
  async cancelThumbnail(requestId: string): Promise<boolean> {
    if (this.session.disposed || !this.activeThumbnailRequests.has(requestId)) return false;
    return this.api.thumbnailCancel({ requestId });
  }

  async createPreview(request: PreviewCreateRequest): Promise<PreviewSnapshot | null> {
    if (this.suspendedValue || this.session.disposed) return null;
    const publication = this.beginPreviewPublication(previewPublicationFromCreateRequest(request));
    const token = this.session.beginRequest();
    const snapshot = await this.api.previewCreate(request);
    if (!this.session.canPublish(token)
      || !this.isLatestPreviewPublication(publication)
      || !previewSnapshotMatches(snapshot, snapshot.previewId, publication)) {
      void this.disposePreviewInternal(snapshot.previewId, true);
      return null;
    }
    this.ownedPreviewIds.add(snapshot.previewId);
    const settledPublication = previewPublicationFromSnapshot(snapshot, publication);
    this.previewPublications.set(snapshot.previewId, settledPublication);
    this.noteLatestPreviewPublication(settledPublication);
    this.previewsValue.set(snapshot.previewId, snapshot);
    this.emit();
    return snapshot;
  }

  async startPreview(previewId: string): Promise<PreviewSnapshot | null> {
    if (this.suspendedValue || this.session.disposed || !this.ownedPreviewIds.has(previewId)) return null;
    const token = this.session.beginRequest();
    const publication = this.previewPublications.get(previewId);
    try {
      const snapshot = await this.api.previewStart({ previewId });
      if (!this.session.canPublish(token) || !this.ownedPreviewIds.has(previewId)) {
        if (this.ownedPreviewIds.has(previewId)) void this.disposePreview(previewId);
        return null;
      }
      if (publication === undefined || !this.acceptPreviewSnapshot(previewId, snapshot, publication)) return null;
      this.previewsValue.set(previewId, snapshot);
      this.emit();
      return snapshot;
    } catch (error) {
      if (!this.session.canPublish(token) && this.ownedPreviewIds.has(previewId)) {
        void this.disposePreview(previewId);
      }
      throw error;
    }
  }

  async snapshotPreview(previewId: string): Promise<PreviewSnapshot | null> {
    if (this.suspendedValue || this.session.disposed || !this.ownedPreviewIds.has(previewId)) return null;
    const token = this.session.beginRequest();
    const publication = this.previewPublications.get(previewId);
    const snapshot = await this.api.previewSnapshot({ previewId });
    if (!this.session.canPublish(token) || !this.ownedPreviewIds.has(previewId)) return null;
    if (publication === undefined || !this.acceptPreviewSnapshot(previewId, snapshot, publication)) return null;
    this.previewsValue.set(previewId, snapshot);
    this.emit();
    return snapshot;
  }

  /**
   * Updates only the native host geometry through the existing Preview
   * snapshot command. The backend revalidates the opaque token, source
   * version and host before touching AppKit; this controller stores no path
   * or native object.
   */
  async updateNativePreviewGeometry(
    previewId: string,
    nativePresentation: PreviewNativePresentation
  ): Promise<PreviewSnapshot | null> {
    if (this.suspendedValue || this.session.disposed || !this.ownedPreviewIds.has(previewId)) return null;
    const token = this.session.beginRequest();
    const publication = this.previewPublications.get(previewId);
    const snapshot = await this.api.previewSnapshot({ previewId, nativePresentation });
    if (!this.session.canPublish(token) || !this.ownedPreviewIds.has(previewId)) return null;
    if (publication === undefined || !this.acceptPreviewSnapshot(previewId, snapshot, publication)) return null;
    this.previewsValue.set(previewId, snapshot);
    this.emit();
    return snapshot;
  }

  /** Retrieves one current Preview asset through the exact opaque tuple. */
  async requestPreviewAsset(request: PreviewAssetRequest): Promise<PreviewAssetArtifact> {
    if (this.suspendedValue || this.session.disposed || !this.ownedPreviewIds.has(request.previewId)) {
      throw new Error("preview_asset_unavailable");
    }
    return this.api.previewAssetRequest(request);
  }

  async switchPreviewSource(request: PreviewSwitchSourceRequest): Promise<PreviewSnapshot | null> {
    if (this.suspendedValue || this.session.disposed || !this.ownedPreviewIds.has(request.previewId)) return null;

    const token = this.session.beginRequest();
    const previousPublication = this.previewPublications.get(request.previewId);
    const hostKind = previousPublication?.hostKind ?? this.previewsValue.get(request.previewId)?.hostKind;
    if (hostKind === undefined) return null;
    const publication = this.beginPreviewPublication(
      previewPublicationFromRequest(request, hostKind),
      request.previewId
    );
    let queue = this.previewSwitchQueues.get(request.previewId);
    if (queue === undefined) {
      queue = {
        inFlight: null,
        pending: null,
        ...(previousPublication === undefined ? {} : { settledPublication: previousPublication })
      };
      this.previewSwitchQueues.set(request.previewId, queue);
    }
    this.previewPublications.set(request.previewId, publication);

    const operationPromise = new Promise<PreviewSnapshot | null>((resolve, reject) => {
      if (queue!.pending !== null) queue!.pending.resolve(null);
      queue!.pending = { request, publication, token, resolve, reject };
    });
    void this.drainPreviewSwitchQueue(request.previewId, queue);
    return operationPromise;
  }

  /** Idempotent single-session cleanup for a presentation-owned Preview. */
  async disposePreview(previewId: string): Promise<boolean> {
    return this.disposePreviewInternal(previewId, false);
  }

  private async disposePreviewInternal(previewId: string, force: boolean): Promise<boolean> {
    const existing = this.previewDisposals.get(previewId);
    if (existing !== undefined) return existing;
    if (!force && !this.ownedPreviewIds.has(previewId)) return false;

    const publication = this.previewPublications.get(previewId);
    this.discardPreviewSwitchQueue(previewId);
    this.ownedPreviewIds.delete(previewId);
    this.previewPublications.delete(previewId);
    if (publication !== undefined
      && samePreviewGeneration(this.latestPreviewPublications.get(publication.hostKind), publication)) {
      this.latestPreviewPublications.delete(publication.hostKind);
    }
    this.previewsValue.delete(previewId);
    this.emit();
    const cleanup = (async () => {
      try {
        await this.api.previewCancel({ previewId });
      } finally {
        await this.api.previewDispose({ previewId });
      }
      return true;
    })();
    const tracked = this.trackCleanup(cleanup) as Promise<boolean>;
    this.previewDisposals.set(previewId, tracked);
    void tracked.then(() => {
      if (this.previewDisposals.get(previewId) === tracked) {
        this.previewDisposals.delete(previewId);
      }
    });
    return tracked;
  }

  /**
   * Suspends disposable current-target work while retaining WorkspaceSession
   * chronology and history-owned Browse refs for a later in-process return.
   */
  async suspend() {
    if (this.session.disposed || this.suspendedValue) return false;
    this.suspendedValue = true;
    this.session.invalidateRequests();
    this.updateBrowseHistoryOwnership();
    const cleanup = this.teardownCurrentTargetWork();
    this.clearPublishedState();
    this.emit();
    const suspension = (async () => {
      await cleanup;
      if (this.session.disposed) return;
      this.finalizeBrowseHistoryOwnership();
      this.emit();
    })();
    this.suspensionPromise = suspension;
    await suspension;
    if (this.suspensionPromise === suspension) this.suspensionPromise = null;
    return true;
  }

  /** Resumes publication for the retained current target without guessing work. */
  async resume() {
    if (this.session.disposed) return false;
    const pendingSuspension = this.suspensionPromise;
    if (pendingSuspension !== null) await pendingSuspension;
    if (this.session.disposed || !this.suspendedValue) return false;
    this.suspendedValue = false;
    this.publishLiveBrowseTargetIfAvailable();
    this.emit();
    return true;
  }

  async dispose(): Promise<void> {
    if (!this.session.dispose()) return;
    const cleanup = this.teardownCurrentTargetWork();
    await cleanup;
    this.disposeAllBrowseSessions();
    const pending = [...this.pendingCleanup];
    await Promise.allSettled(pending);
    this.disposeAllBrowseSessions();
    this.suspendedValue = false;
    this.clearPublishedState();
    this.emit();
  }

  private async restoreCurrentHistoryBrowse(): Promise<BrowseOpenResponse | null> {
    const target = this.session.currentTarget;
    const locator = this.session.serializeRestoreLocator();
    if (target?.kind !== "browse" || locator?.kind !== "browse") return null;
    const token = this.session.beginRequest();
    const response = await this.api.browseRestore({ locator });
    if (!this.session.canPublish(token)) {
      this.trackCleanup(this.cleanupCall(() => this.api.browseDispose({ sessionId: response.sessionId })));
      return null;
    }
    return this.publishBrowseAdmission(response, locator, token, true);
  }

  private async publishBrowseAdmission(
    response: BrowseOpenResponse,
    restoreLocator: WorkspaceRestoreLocator | undefined,
    token: WorkspaceRequestToken,
    replaceHistorySlot: boolean
  ): Promise<BrowseOpenResponse | null> {
    const target: NavigationTarget = {
      kind: "browse",
      location: response.location.ref,
      pathRef: response.rootPathRef
    };
    if (!this.session.canPublish(token)) {
      this.trackCleanup(this.cleanupCall(() => this.api.browseDispose({ sessionId: response.sessionId })));
      return null;
    }
    const previousEpoch = this.session.requestEpoch;
    const rebound = replaceHistorySlot
      ? this.session.replaceCurrentTarget(
        target,
        restoreLocator === undefined ? {} : { restoreLocator }
      )
      : this.session.navigate(
        target,
        restoreLocator === undefined ? {} : { restoreLocator }
      );
    if (!rebound) {
      this.trackCleanup(this.cleanupCall(() => this.api.browseDispose({ sessionId: response.sessionId })));
      return null;
    }
    const admissionEpoch = this.session.requestEpoch;

    this.updateBrowseHistoryOwnership();
    const cleanup = admissionEpoch === previousEpoch
      ? Promise.resolve()
      : this.teardownCurrentTargetWork();
    this.clearPublishedState();
    await cleanup;
    if (!this.session.isEpochCurrent(admissionEpoch)) {
      this.trackCleanup(this.cleanupCall(() => this.api.browseDispose({ sessionId: response.sessionId })));
      return null;
    }
    this.finalizeBrowseHistoryOwnership();

    const owner: OwnedBrowseSession = {
      response,
      ...(restoreLocator === undefined ? {} : { restoreLocator }),
      pages: new Map(),
      pathRefs: new Map([[response.rootPathRef.id, response.rootPathRef]]),
      historyPathRefs: new Map(),
      promotedPathRefs: new Set([response.rootPathRef.id]),
      pendingPathRetention: new Map(),
      unavailable: false,
      enumerations: new Map()
    };
    this.ownedBrowseSessions.set(response.sessionId, owner);
    this.updateBrowseHistoryOwnership();
    this.finalizeBrowseHistoryOwnership();
    this.browseResponse = response;
    if (response.initialPage) this.publishPage(response.initialPage);
    this.emit();
    return response;
  }

  private currentBrowseTarget() {
    const target = this.session.currentTarget;
    return target?.kind === "browse" ? target : null;
  }

  private publishPage(page: BrowsePage) {
    const owner = this.ownedBrowseSessions.get(page.sessionId);
    if (owner === undefined) {
      this.releasePageLater(page);
      return;
    }
    // A next-page response is a distinct backend-owned batch even when it
    // shares the same enumeration/request identity. Keep every published
    // batch until enumeration supersede, target teardown, session disposal or
    // an explicit bounded eviction point; releasing the previous batch here
    // would invalidate entries still rendered by the workspace.
    owner.pages.set(this.keyForPage(page), page);
    owner.enumerations.set(page.enumerationId, enumerationForPage(page));
    for (const entry of page.entries) {
      if (entry.pathRef !== undefined) owner.pathRefs.set(entry.pathRef.id, entry.pathRef);
    }
    this.currentPage = page;
    this.emit();
  }

  private releasePageLater(page: BrowsePage) {
    return this.trackCleanup(this.cleanupCall(() => this.api.browseReleasePage({ page })));
  }

  private keyForPage(page: BrowsePage) {
    const existing = this.pageKeys.get(page);
    if (existing !== undefined) return existing;
    this.nextPageKey += 1;
    const key = `${page.enumerationId}:${page.requestId}:batch-${this.nextPageKey}`;
    this.pageKeys.set(page, key);
    return key;
  }

  private historyBrowsePathRequirements() {
    const requirements = new Map<string, Map<string, BrowsePathRef>>();
    for (const target of this.session.getState().history) {
      if (target.kind !== "browse" || target.location.kind !== "ephemeral") continue;
      const paths = requirements.get(target.location.browseSessionId) ?? new Map();
      paths.set(target.pathRef.id, target.pathRef);
      requirements.set(target.location.browseSessionId, paths);
    }
    return requirements;
  }

  /**
   * Reconciles WorkspaceSession history with process-local Browse ownership.
   * This phase only updates history pins and starts the minimal backend retain
   * seam; it deliberately does not dispose sessions until disposable work has
   * been torn down.
   */
  private updateBrowseHistoryOwnership() {
    const requirements = this.historyBrowsePathRequirements();
    for (const owner of this.ownedBrowseSessions.values()) {
      owner.historyPathRefs.clear();
      const required = requirements.get(owner.response.sessionId);
      if (required === undefined) continue;
      for (const [pathId, requestedPathRef] of required) {
        const pathRef = owner.pathRefs.get(pathId) ?? requestedPathRef;
        owner.pathRefs.set(pathId, pathRef);
        owner.historyPathRefs.set(pathId, pathRef);
        if (!owner.promotedPathRefs.has(pathId)) this.ensurePathRetention(owner, pathRef);
      }
    }
  }

  /** Release path/session handles only after current-target work is gone. */
  private finalizeBrowseHistoryOwnership() {
    const requirements = this.historyBrowsePathRequirements();
    for (const owner of [...this.ownedBrowseSessions.values()]) {
      const required = requirements.get(owner.response.sessionId);
      if (required === undefined) {
        this.disposeOwnedBrowseSession(owner);
        continue;
      }

      const pagePathIds = new Set<string>();
      for (const page of owner.pages.values()) {
        for (const entry of page.entries) {
          if (entry.pathRef !== undefined) pagePathIds.add(entry.pathRef.id);
        }
      }
      for (const [pathId, pathRef] of [...owner.pathRefs.entries()]) {
        if (required.has(pathId) || pagePathIds.has(pathId)) continue;
        owner.pathRefs.delete(pathId);
        owner.promotedPathRefs.delete(pathId);
        owner.historyPathRefs.delete(pathId);
        this.trackCleanup(this.cleanupCall(() => this.api.browseReleasePath({
          sessionId: owner.response.sessionId,
          pathRef
        })));
      }
    }
  }

  private ensurePathRetention(owner: OwnedBrowseSession, pathRef: BrowsePathRef) {
    if (owner.promotedPathRefs.has(pathRef.id)) return Promise.resolve(true);
    const existing = owner.pendingPathRetention.get(pathRef.id);
    if (existing !== undefined) return existing;
    if (owner.unavailable) return Promise.resolve(false);

    const operation = this.api.browseRetainPath({
      sessionId: owner.response.sessionId,
      pathRef
    }).then(
      () => {
        owner.promotedPathRefs.add(pathRef.id);
        return true;
      },
      () => {
        owner.unavailable = true;
        return false;
      }
    );
    const tracked = this.trackCleanup(operation) as Promise<boolean>;
    owner.pendingPathRetention.set(pathRef.id, tracked);
    void tracked.then(() => {
      if (owner.pendingPathRetention.get(pathRef.id) === tracked) {
        owner.pendingPathRetention.delete(pathRef.id);
      }
    });
    return tracked;
  }

  private waitForPathRetention(owner: OwnedBrowseSession, pathId: string) {
    if (!owner.historyPathRefs.has(pathId)) return !owner.unavailable;
    if (owner.promotedPathRefs.has(pathId)) return true;
    const pathRef = owner.pathRefs.get(pathId);
    if (pathRef === undefined) return false;
    return this.ensurePathRetention(owner, pathRef);
  }

  private pageRetentionPromises(owner: OwnedBrowseSession, page: BrowsePage) {
    const promises = new Set<Promise<boolean>>();
    for (const entry of page.entries) {
      const pathRef = entry.pathRef;
      if (pathRef === undefined
        || !owner.historyPathRefs.has(pathRef.id)
        || owner.promotedPathRefs.has(pathRef.id)) {
        continue;
      }
      promises.add(this.ensurePathRetention(owner, pathRef));
    }
    return [...promises];
  }

  private async releasePagesForSupersededEnumerations(
    owner: OwnedBrowseSession,
    currentEnumerationId: string
  ) {
    const releases: Array<{ page: BrowsePage; retention: Promise<boolean>[] }> = [];
    for (const [key, page] of [...owner.pages.entries()]) {
      if (page.enumerationId === currentEnumerationId) continue;
      owner.pages.delete(key);
      releases.push({ page, retention: this.pageRetentionPromises(owner, page) });
    }
    for (const enumerationId of [...owner.enumerations.keys()]) {
      if (enumerationId !== currentEnumerationId) owner.enumerations.delete(enumerationId);
    }
    await Promise.all(releases.map(async ({ page, retention }) => {
      await Promise.all(retention);
      await this.cleanupCall(() => this.api.browseReleasePage({ page }));
    }));
  }

  /** Teardown disposable target work while preserving history-owned sessions. */
  private teardownCurrentTargetWork(): Promise<unknown> {
    const pendingEnumerations = [...this.pendingEnumerations.values()];
    this.pendingEnumerations.clear();
    const enumerationCalls = new Map<string, () => Promise<unknown>>();
    for (const pending of pendingEnumerations) {
      const key = pending.enumeration === undefined
        ? `request:${pending.sessionId}:${pending.requestId}`
        : `enumeration:${pending.enumeration.enumerationId}`;
      enumerationCalls.set(key, () => this.api.browseCancel(
        pending.enumeration === undefined
          ? { sessionId: pending.sessionId, requestId: pending.requestId }
          : { sessionId: pending.sessionId, enumeration: pending.enumeration }
      ));
    }

    const pageReleases: Array<{
      page: BrowsePage;
      retention: Promise<boolean>[];
    }> = [];
    for (const owner of this.ownedBrowseSessions.values()) {
      for (const [key, page] of [...owner.pages.entries()]) {
        owner.pages.delete(key);
        pageReleases.push({ page, retention: this.pageRetentionPromises(owner, page) });
      }
      for (const enumeration of owner.enumerations.values()) {
        enumerationCalls.set(`enumeration:${enumeration.enumerationId}`, () => this.api.browseCancel({
          sessionId: owner.response.sessionId,
          enumeration
        }));
      }
      owner.enumerations.clear();
    }

    const monitorIds = [...this.ownedMonitors.keys()];
    this.ownedMonitors.clear();
    const thumbnailIds = [...this.activeThumbnailRequests];
    this.activeThumbnailRequests.clear();
    const previewIds = [...this.ownedPreviewIds];
    this.ownedPreviewIds.clear();

    const cleanup = (async () => {
      // A page-originated history path must be pinned before cancellation
      // invalidates its enumeration entries or release_page drops ownership.
      const pageRetention = pageReleases.flatMap(({ retention }) => retention);
      if (pageRetention.length > 0) await Promise.all(pageRetention);
      await Promise.all([...enumerationCalls.values()].map((call) => this.cleanupCall(call)));
      await Promise.all(pageReleases.map(({ page }) => this.cleanupCall(
        () => this.api.browseReleasePage({ page })
      )));
      await Promise.all(monitorIds.map((monitorId) => this.cleanupCall(
        () => this.api.changeDispose({ monitorId })
      )));
      await Promise.all(thumbnailIds.map((requestId) => this.cleanupCall(
        () => this.api.thumbnailCancel({ requestId })
      )));
      await Promise.all(previewIds.map((previewId) => this.disposePreviewInternal(previewId, true)));
    })();
    return this.trackCleanup(cleanup);
  }

  private disposeOwnedBrowseSession(owner: OwnedBrowseSession) {
    if (this.ownedBrowseSessions.get(owner.response.sessionId) !== owner) return;
    this.ownedBrowseSessions.delete(owner.response.sessionId);
    const pages = [...owner.pages.values()];
    const enumerations = [...owner.enumerations.values()];
    const pathRefs = [...owner.pathRefs.values()];
    const pathRetentions = [...owner.pendingPathRetention.values()];
    owner.pages.clear();
    owner.enumerations.clear();
    owner.pathRefs.clear();
    owner.historyPathRefs.clear();
    owner.promotedPathRefs.clear();
    owner.pendingPathRetention.clear();

    const cleanup = (async () => {
      await Promise.all(pathRetentions);
      await Promise.all(enumerations.map((enumeration) => this.cleanupCall(
        () => this.api.browseCancel({ sessionId: owner.response.sessionId, enumeration })
      )));
      await Promise.all(pages.map((page) => this.cleanupCall(
        () => this.api.browseReleasePage({ page })
      )));
      await Promise.all(pathRefs.map((pathRef) => this.cleanupCall(
        () => this.api.browseReleasePath({
          sessionId: owner.response.sessionId,
          pathRef
        })
      )));
      await this.cleanupCall(() => this.api.browseDispose({
        sessionId: owner.response.sessionId
      }));
    })();
    this.trackCleanup(cleanup);
  }

  private disposeAllBrowseSessions() {
    for (const owner of [...this.ownedBrowseSessions.values()]) {
      this.disposeOwnedBrowseSession(owner);
    }
  }

  private publishLiveBrowseTargetIfAvailable() {
    const target = this.currentBrowseTarget();
    if (target === null || target.location.kind !== "ephemeral") return null;
    const owner = this.ownedBrowseSessions.get(target.location.browseSessionId);
    if (owner === undefined
      || owner.unavailable
      || !owner.pathRefs.has(target.pathRef.id)
      || !owner.historyPathRefs.has(target.pathRef.id)) {
      return null;
    }
    this.browseResponse = owner.response;
    return owner.response;
  }

  private acceptPreviewSnapshot(
    previewId: string,
    snapshot: PreviewSnapshot,
    capturedPublication: PreviewPublication
  ) {
    if (snapshot.previewId !== previewId) return false;
    const current = this.previewPublications.get(previewId);
    if (current === undefined
      || !samePreviewGeneration(current, capturedPublication)
      || !this.isLatestPreviewPublication(capturedPublication)
      || !previewSnapshotMatches(snapshot, previewId, capturedPublication)) return false;
    const settledPublication = previewPublicationFromSnapshot(snapshot, capturedPublication);
    this.previewPublications.set(previewId, settledPublication);
    this.noteLatestPreviewPublication(settledPublication);
    return true;
  }

  private beginPreviewPublication(
    draft: PreviewPublicationDraft,
    exceptPreviewId?: string
  ) {
    const publication: PreviewPublication = {
      ...draft,
      generation: this.nextPreviewPublicationGeneration(draft.hostKind)
    };
    this.noteLatestPreviewPublication(publication);
    for (const [previewId, current] of [...this.previewPublications.entries()]) {
      if (previewId === exceptPreviewId || current.hostKind !== publication.hostKind) continue;
      void this.disposePreview(previewId);
    }
    return publication;
  }

  private noteLatestPreviewPublication(publication: PreviewPublication) {
    const current = this.latestPreviewPublications.get(publication.hostKind);
    if (current === undefined || publication.generation >= current.generation) {
      this.latestPreviewPublications.set(publication.hostKind, publication);
    }
  }

  private isLatestPreviewPublication(publication: PreviewPublication) {
    return samePreviewGeneration(this.latestPreviewPublications.get(publication.hostKind), publication);
  }

  private nextPreviewPublicationGeneration(hostKind: PreviewHostKind) {
    const current = this.previewPublicationGenerations.get(hostKind) ?? 0;
    if (current >= Number.MAX_SAFE_INTEGER) {
      throw new Error("Preview publication generation exhausted");
    }
    const next = current + 1;
    this.previewPublicationGenerations.set(hostKind, next);
    return next;
  }

  private async drainPreviewSwitchQueue(previewId: string, queue: PreviewSwitchQueue) {
    if (queue.inFlight !== null) return;
    const operation = queue.pending;
    if (operation === null) {
      if (this.previewSwitchQueues.get(previewId) === queue) this.previewSwitchQueues.delete(previewId);
      return;
    }

    queue.pending = null;
    queue.inFlight = operation;
    try {
      const snapshot = await this.api.previewSwitchSource(operation.request);
      const matches = previewSnapshotMatches(snapshot, previewId, operation.publication);
      if (matches) queue.settledPublication = previewPublicationFromSnapshot(snapshot, operation.publication);

      const current = queue.pending === null
        && this.session.canPublish(operation.token)
        && this.ownedPreviewIds.has(previewId)
        && this.isLatestPreviewPublication(operation.publication)
        && samePreviewGeneration(this.previewPublications.get(previewId), operation.publication)
        && matches;
      if (!current) {
        operation.resolve(null);
      } else {
        this.ownedPreviewIds.add(previewId);
        this.previewsValue.set(previewId, snapshot);
        const settledPublication = previewPublicationFromSnapshot(snapshot, operation.publication);
        this.previewPublications.set(previewId, settledPublication);
        this.noteLatestPreviewPublication(settledPublication);
        this.emit();
        operation.resolve(snapshot);
      }
    } catch (error) {
      const current = queue.pending === null
        && this.session.canPublish(operation.token)
        && this.ownedPreviewIds.has(previewId)
        && this.isLatestPreviewPublication(operation.publication)
        && samePreviewGeneration(this.previewPublications.get(previewId), operation.publication);
      if (!current) {
        operation.resolve(null);
      } else {
        this.restorePreviewSwitchPublication(previewId, queue, operation.publication);
        operation.reject(error);
      }
    } finally {
      if (queue.inFlight === operation) queue.inFlight = null;

      if (!this.session.canPublish(operation.token) || !this.ownedPreviewIds.has(previewId)) {
        const pending = this.takePendingPreviewSwitch(queue);
        if (pending !== null) {
          pending.resolve(null);
        }
        if (this.ownedPreviewIds.has(previewId)) {
          void this.disposePreview(previewId);
        }
        if (this.previewSwitchQueues.get(previewId) === queue) this.previewSwitchQueues.delete(previewId);
        return;
      }

      if (queue.pending !== null) {
        void this.drainPreviewSwitchQueue(previewId, queue);
      } else if (this.previewSwitchQueues.get(previewId) === queue) {
        this.previewSwitchQueues.delete(previewId);
      }
    }
  }

  private restorePreviewSwitchPublication(
    previewId: string,
    queue: PreviewSwitchQueue,
    failedPublication: PreviewPublication
  ) {
    const current = this.previewPublications.get(previewId);
    if (!samePreviewGeneration(current, failedPublication)
      || !this.isLatestPreviewPublication(failedPublication)) return;

    if (queue.settledPublication === undefined) {
      this.previewPublications.delete(previewId);
      if (samePreviewGeneration(this.latestPreviewPublications.get(failedPublication.hostKind), failedPublication)) {
        this.latestPreviewPublications.delete(failedPublication.hostKind);
      }
    } else {
      this.previewPublications.set(previewId, queue.settledPublication);
      if (samePreviewGeneration(this.latestPreviewPublications.get(failedPublication.hostKind), failedPublication)) {
        this.latestPreviewPublications.delete(failedPublication.hostKind);
      }
      this.noteLatestPreviewPublication(queue.settledPublication);
    }
    this.emit();
  }

  private discardPreviewSwitchQueue(previewId: string) {
    const queue = this.previewSwitchQueues.get(previewId);
    if (queue === undefined) return;
    const pending = this.takePendingPreviewSwitch(queue);
    if (pending !== null) pending.resolve(null);
    if (queue.inFlight === null) this.previewSwitchQueues.delete(previewId);
  }

  private takePendingPreviewSwitch(queue: PreviewSwitchQueue) {
    const pending = queue.pending;
    queue.pending = null;
    return pending;
  }

  private clearPublishedState() {
    this.browseResponse = null;
    this.currentPage = null;
    this.changeResponse = null;
    this.pendingChangeValue = null;
    this.eligibilityValue = null;
    for (const previewId of [...this.previewSwitchQueues.keys()]) this.discardPreviewSwitchQueue(previewId);
    this.previewsValue.clear();
    this.previewPublications.clear();
    this.latestPreviewPublications.clear();
  }

  private trackCleanup(operation: Promise<unknown>): Promise<unknown> {
    const tracked = operation.catch(() => undefined);
    this.pendingCleanup.add(tracked);
    void tracked.then(() => this.pendingCleanup.delete(tracked));
    return tracked;
  }

  private cleanupCall(call: () => Promise<unknown>): Promise<unknown> {
    return Promise.resolve().then(call).catch(() => undefined);
  }

  private emit() {
    const state = this.getState();
    for (const listener of this.listeners) listener(state);
  }
}

function browseRestoreLocator(request: BrowseOpenRequest): WorkspaceRestoreLocator {
  return {
    kind: "browse",
    platform: request.platform,
    routingHint: request.routingHint,
    ...(request.displayHint === undefined ? {} : { displayHint: request.displayHint })
  };
}

function previewPublicationFromCreateRequest(request: PreviewCreateRequest): PreviewPublicationDraft {
  return {
    requestId: request.requestId,
    source: request.source,
    hostKind: request.hostKind
  };
}

function previewPublicationFromRequest(
  request: PreviewSwitchSourceRequest,
  hostKind: PreviewHostKind
): PreviewPublicationDraft {
  return {
    previewId: request.previewId,
    requestId: request.requestId,
    source: request.source,
    hostKind
  };
}

function previewPublicationFromSnapshot(
  snapshot: PreviewSnapshot,
  publication: PreviewPublication
): PreviewPublication {
  return {
    generation: publication.generation,
    previewId: snapshot.previewId,
    requestId: snapshot.requestId,
    source: snapshot.source,
    hostKind: snapshot.hostKind,
    ...(snapshot.sourceVersion === undefined ? {} : { sourceVersion: snapshot.sourceVersion })
  };
}

function previewSnapshotMatches(
  snapshot: PreviewSnapshot,
  previewId: string,
  publication: PreviewPublication
) {
  return snapshot.previewId === previewId
    && snapshot.requestId === publication.requestId
    && snapshot.hostKind === publication.hostKind
    && samePreviewSource(snapshot.source, publication.source)
    && (publication.sourceVersion === undefined || snapshot.sourceVersion === publication.sourceVersion);
}

function samePreviewGeneration(left: PreviewPublication | undefined, right: PreviewPublication) {
  return left !== undefined
    && left.hostKind === right.hostKind
    && left.generation === right.generation;
}

function samePreviewSource(left: PreviewSourceRef, right: PreviewSourceRef) {
  if (left.kind !== right.kind) return false;
  if (left.kind === "managed" && right.kind === "managed") return left.fileId === right.fileId;
  if (left.kind === "ephemeral" && right.kind === "ephemeral") {
    return left.browseSessionId === right.browseSessionId && left.entryId === right.entryId;
  }
  return left.kind === "host_provided" && right.kind === "host_provided" && left.hostToken === right.hostToken;
}

function enumerationForPage(page: BrowsePage): BrowseEnumerationRef {
  return {
    sessionId: page.sessionId,
    requestId: page.requestId,
    enumerationId: page.enumerationId
  };
}
