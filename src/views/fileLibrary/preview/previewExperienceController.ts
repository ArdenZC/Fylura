import type {
  ContentReadEligibility,
  PreviewAssetArtifact,
  PreviewAssetRequest,
  PreviewNativePresentation,
  PreviewSourceRef,
  PreviewHostKind,
  PreviewSessionState,
  PreviewSnapshot,
  PreviewTerminalCondition,
  PreviewWarning
} from "../../../types/fileWorkspace";
import type { FileWorkspaceController } from "../../../fileWorkspace";
import type { PreviewSourceProjection } from "./previewSource";
import {
  previewSiblingNavigationState,
  type PreviewSiblingDirection,
  type PreviewSiblingNavigationProjection,
  type PreviewSiblingNavigationState
} from "./previewSiblingNavigation";

export type PreviewExperiencePhase =
  | "closed"
  | "resolving"
  | "loading"
  | "content"
  | "metadata_fallback"
  | "no_source"
  | "source_unavailable"
  | "materialization_required"
  | "permission_denied"
  | "identity_changed"
  | "cancelled"
  | "unsupported_representation"
  | "error";

/**
 * User-visible state for the rendered Preview surface. The lifecycle phase
 * remains the controller's compatibility contract; this projection makes
 * completeness and provider fallback reasons observable without turning the
 * renderer into a second authority.
 */
export type PreviewPresentationState =
  | "closed"
  | "no_source"
  | "resolving"
  | "loading"
  | "ready"
  | "partial"
  | "metadata_fallback"
  | "unsupported"
  | "unavailable"
  | "failed"
  | "permission_required"
  | "materialization_required"
  | "identity_changed"
  | "cancelled";

export type PreviewImagePresentationState = Extract<
  PreviewPresentationState,
  "loading" | "ready" | "unavailable" | "unsupported" | "failed"
>;

export type PreviewFallbackState = "metadata" | "unsupported" | "failed";

export type PreviewExperienceHost = "floating" | "pinned";

export interface PreviewPinnedHandoff {
  readonly fromHost: "zen_floating";
  readonly toHost: "zen_pinned";
  /** The superseded Floating session captured when Pin was invoked. */
  readonly previewId: string;
  /** The bounded staging session accepted by the Context owner. */
  readonly stagedPreviewId: string;
  readonly stagedSnapshot: PreviewSnapshot;
  readonly source: Extract<PreviewSourceRef, { kind: "managed" | "ephemeral" }>;
  readonly sourceKey: string;
  readonly frontendEpoch: number;
}

export type PreviewPinnedHandoffHandler = (handoff: PreviewPinnedHandoff) => boolean | Promise<boolean>;

export interface PreviewExperienceState {
  readonly visible: boolean;
  readonly host: PreviewExperienceHost | null;
  readonly frontendEpoch: number;
  readonly detailsOpen: boolean;
  readonly source: PreviewSourceProjection | null;
  readonly previewId: string | null;
  readonly snapshot: PreviewSnapshot | null;
  readonly phase: PreviewExperiencePhase;
  readonly navigation: PreviewSiblingNavigationState | null;
  readonly navigationBusy: boolean;
}

export type PreviewOpenPreparation = () => void;

export interface PreviewSpaceEvent {
  readonly altKey: boolean;
  readonly defaultPrevented?: boolean;
  readonly isComposing?: boolean;
  readonly repeat?: boolean;
  readonly target: EventTarget | null;
}

const CLOSED_STATE: PreviewExperienceState = {
  visible: false,
  host: null,
  frontendEpoch: 0,
  detailsOpen: false,
  source: null,
  previewId: null,
  snapshot: null,
  phase: "closed",
  navigation: null,
  navigationBusy: false
};

const PREVIEW_SNAPSHOT_OBSERVATION_INTERVAL_MS = 250;
const MAX_PREVIEW_SNAPSHOT_OBSERVATIONS = 16;

interface PendingPreviewSnapshotObservation {
  readonly epoch: number;
  readonly source: PreviewSourceProjection;
  readonly previewId: string;
  requestCount: number;
  inFlight: boolean;
  timer: ReturnType<typeof setTimeout> | null;
}

/**
 * The sole W3-02 renderer owner. It owns only disposable UI/epoch state and
 * delegates Preview lifecycle to FileWorkspaceController, which remains the
 * backend handle owner.
 */
export class PreviewExperienceController {
  private readonly workspace: FileWorkspaceController;
  private readonly listeners = new Set<(state: PreviewExperienceState) => void>();
  private stateValue: PreviewExperienceState = CLOSED_STATE;
  private prepareOpenValue: PreviewOpenPreparation;
  private pinHandoffValue: PreviewPinnedHandoffHandler;
  private originFocus: HTMLElement | null = null;
  private disposedValue = false;
  private nextRequest = 0;
  private siblingNavigationValue: PreviewSiblingNavigationProjection | null = null;
  private navigationBusyValue = false;
  private pinnedSiblingObservationPending = false;
  private pinHandoffPromise: Promise<boolean> | null = null;
  private previewObservationValue: PendingPreviewSnapshotObservation | null = null;

  constructor(
    workspace: FileWorkspaceController,
    prepareOpen: PreviewOpenPreparation = () => undefined,
    onPinHandoff: PreviewPinnedHandoffHandler = () => true
  ) {
    this.workspace = workspace;
    this.prepareOpenValue = prepareOpen;
    this.pinHandoffValue = onPinHandoff;
  }

  getState() {
    return this.stateWithNavigation();
  }

  subscribe(listener: (state: PreviewExperienceState) => void) {
    this.listeners.add(listener);
    listener(this.stateWithNavigation());
    return () => this.listeners.delete(listener);
  }

  setPrepareOpen(prepareOpen: PreviewOpenPreparation) {
    this.prepareOpenValue = prepareOpen;
  }

  setPinHandoff(onPinHandoff: PreviewPinnedHandoffHandler) {
    this.pinHandoffValue = onPinHandoff;
  }

  /** Returns false when the event is guarded and therefore must not be consumed. */
  handleSpace(
    source: PreviewSourceProjection | null,
    trigger: HTMLElement | null,
    event?: PreviewSpaceEvent
  ) {
    if (event !== undefined && !isPreviewWorkspaceSpaceEligible(event)) return false;
    if (this.stateValue.visible) {
      if (this.stateValue.host === "pinned") return false;
      this.close("space");
      return true;
    }
    if (source === null) return false;
    return this.open(source, trigger);
  }

  open(source: PreviewSourceProjection | null, trigger: HTMLElement | null) {
    if (this.disposedValue || source === null) return false;
    if (this.stateValue.visible) {
      if (this.stateValue.host === "pinned") return true;
      this.publishSource(source);
      return true;
    }

    this.prepareOpenValue();
    this.originFocus = isRestorableFocusTarget(trigger) ? trigger : document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const epoch = this.stateValue.frontendEpoch + 1;
    this.stateValue = {
      visible: true,
      host: "floating",
      frontendEpoch: epoch,
      detailsOpen: false,
      source,
      previewId: null,
      snapshot: null,
      phase: "resolving",
      navigation: null,
      navigationBusy: false
    };
    this.stopPreviewObservation();
    this.emit();
    void this.createAndStart(epoch, source);
    return true;
  }

  /** Reconciles the current loaded source without changing host identity. */
  observeSource(source: PreviewSourceProjection | null) {
    if (!this.stateValue.visible || sameSource(this.stateValue.source, source)) return;
    if (this.stateValue.host === "pinned") {
      if (!this.pinnedSiblingObservationPending) return;
      this.pinnedSiblingObservationPending = false;
    }
    this.publishSource(source);
  }

  setSiblingNavigation(navigation: PreviewSiblingNavigationProjection | null) {
    this.siblingNavigationValue = navigation;
    this.emit();
  }

  requestPreviewAsset(request: PreviewAssetRequest): Promise<PreviewAssetArtifact> {
    return this.workspace.requestPreviewAsset(request);
  }

  updateNativePreviewGeometry(
    previewId: string,
    presentation: PreviewNativePresentation
  ): Promise<PreviewSnapshot | null> {
    if (
      this.disposedValue
      || !this.stateValue.visible
      || this.stateValue.previewId !== previewId
    ) {
      return Promise.resolve(null);
    }
    const epoch = this.stateValue.frontendEpoch;
    const source = this.stateValue.source;
    if (source === null) return Promise.resolve(null);
    return this.workspace.updateNativePreviewGeometry(previewId, presentation).then((snapshot) => {
      if (
        snapshot === null
        || !this.isCurrent(epoch, source)
        || this.stateValue.previewId !== previewId
        || snapshot.previewId !== previewId
      ) {
        return null;
      }
      this.publishSnapshot(epoch, source, snapshot);
      return snapshot;
    });
  }

  async moveSibling(direction: PreviewSiblingDirection) {
    const navigation = this.siblingNavigationValue;
    const state = this.stateWithNavigation();
    if (navigation === null || state.navigation === null || this.navigationBusyValue) return false;
    const available = direction === "previous"
      ? state.navigation.previousAvailable
      : state.navigation.nextAvailable;
    if (!available) return false;

    this.navigationBusyValue = true;
    if (this.stateValue.host === "pinned") this.pinnedSiblingObservationPending = true;
    this.emit();
    try {
      const moved = await navigation.move(direction);
      if (!moved) this.pinnedSiblingObservationPending = false;
      return moved;
    } catch (error) {
      this.pinnedSiblingObservationPending = false;
      throw error;
    } finally {
      this.navigationBusyValue = false;
      this.emit();
    }
  }

  setDetailsOpen(detailsOpen: boolean) {
    if (!this.stateValue.visible || this.stateValue.detailsOpen === detailsOpen) return;
    this.stateValue = { ...this.stateValue, detailsOpen };
    this.emit();
  }

  pin() {
    if (this.disposedValue
      || !this.stateValue.visible
      || this.stateValue.host !== "floating"
      || this.stateValue.previewId === null
      || this.stateValue.source === null) {
      return Promise.resolve(false);
    }

    if (this.pinHandoffPromise !== null) return this.pinHandoffPromise;

    const operation = this.performPin().catch(() => false);
    this.pinHandoffPromise = operation;
    void operation.then(() => {
      if (this.pinHandoffPromise === operation) this.pinHandoffPromise = null;
    });
    return operation;
  }

  private async performPin() {
    const captured = this.stateValue;
    const capturedPreviewId = captured.previewId;
    const capturedSource = captured.source;
    if (capturedPreviewId === null || capturedSource === null || captured.host !== "floating") return false;

    const staged = await this.workspace.createPreview({
      requestId: this.requestId(captured.frontendEpoch),
      source: capturedSource.previewSource,
      hostKind: "zen_pinned"
    });
    if (staged === null) return false;

    if (staged.hostKind !== "zen_pinned" || !this.isCapturedFloating(captured)) {
      await this.workspace.disposePreview(staged.previewId);
      return false;
    }

    const handoff: PreviewPinnedHandoff = {
      fromHost: "zen_floating",
      toHost: "zen_pinned",
      previewId: capturedPreviewId,
      stagedPreviewId: staged.previewId,
      stagedSnapshot: staged,
      source: capturedSource.previewSource,
      sourceKey: capturedSource.key,
      frontendEpoch: captured.frontendEpoch
    };
    let accepted = false;
    try {
      accepted = await this.pinHandoffValue(handoff);
    } catch {
      accepted = false;
    }
    if (!accepted || !this.isCapturedFloating(captured)) {
      await this.workspace.disposePreview(staged.previewId);
      return false;
    }

    const nextEpoch = captured.frontendEpoch + 1;
    this.stateValue = {
      ...captured,
      host: "pinned",
      frontendEpoch: nextEpoch,
      previewId: staged.previewId,
      snapshot: staged,
      phase: phaseForSnapshot(staged)
    };
    this.stopPreviewObservation();
    this.emit();
    void this.workspace.disposePreview(capturedPreviewId);
    void this.startCommittedPreview(nextEpoch, capturedSource, staged.previewId);
    return true;
  }

  unpin() {
    if (this.disposedValue
      || !this.stateValue.visible
      || this.stateValue.host !== "pinned"
      || this.stateValue.source === null) {
      return false;
    }
    const captured = this.stateValue;
    const previousPreviewId = captured.previewId;
    const source = captured.source;
    if (source === null) return false;
    const nextEpoch = captured.frontendEpoch + 1;
    this.pinnedSiblingObservationPending = false;
    this.stopPreviewObservation();
    this.stateValue = {
      ...captured,
      host: "floating",
      frontendEpoch: nextEpoch,
      previewId: null,
      snapshot: null,
      phase: "resolving"
    };
    this.emit();
    if (previousPreviewId !== null) void this.workspace.disposePreview(previousPreviewId);
    void this.createAndStart(nextEpoch, source);
    return true;
  }

  close(reason: "space" | "escape" | "button" | "source_unavailable" | "unpin" | "dispose" = "button") {
    if (reason === "unpin") return this.unpin();
    this.stopPreviewObservation();
    if (!this.stateValue.visible) return false;
    const previewId = this.stateValue.previewId;
    this.pinnedSiblingObservationPending = false;
    this.siblingNavigationValue = null;
    this.stateValue = {
      ...CLOSED_STATE,
      frontendEpoch: this.stateValue.frontendEpoch + 1
    };
    this.emit();
    if (previewId !== null) void this.workspace.disposePreview(previewId);
    return true;
  }

  /** Release the existing preview owner before a confirmed namespace operation. */
  async closeForFileOperation() {
    const previewId = this.stateValue.previewId;
    this.close("button");
    if (previewId !== null) await this.workspace.disposePreview(previewId);
  }

  restoreFocusTarget() {
    if (isRestorableFocusTarget(this.originFocus)) return this.originFocus;
    return document.querySelector<HTMLElement>(
      '[data-shared-file-list="true"], [data-shared-file-grid="true"]'
    );
  }

  async settle() {
    await Promise.resolve();
    await Promise.resolve();
  }

  async dispose() {
    if (this.disposedValue) return false;
    this.disposedValue = true;
    this.close("dispose");
    this.listeners.clear();
    return true;
  }

  private publishSource(source: PreviewSourceProjection | null) {
    this.stopPreviewObservation();
    const previousPreviewId = this.stateValue.previewId;
    const epoch = this.stateValue.frontendEpoch + 1;
    this.stateValue = {
      ...this.stateValue,
      frontendEpoch: epoch,
      detailsOpen: false,
      source,
      previewId: source === null ? null : previousPreviewId,
      snapshot: null,
      phase: source === null ? "no_source" : "resolving"
    };
    this.emit();
    if (source === null) {
      if (previousPreviewId !== null) void this.workspace.disposePreview(previousPreviewId);
      return;
    }
    if (previousPreviewId === null) {
      void this.createAndStart(epoch, source);
      return;
    }
    void this.switchAndStart(epoch, previousPreviewId, source);
  }

  private async createAndStart(epoch: number, source: PreviewSourceProjection) {
    const requestId = this.requestId(epoch);
    const hostKind: PreviewHostKind = this.stateValue.host === "pinned" ? "zen_pinned" : "zen_floating";
    try {
      const created = await this.workspace.createPreview({
        requestId,
        source: source.previewSource,
        hostKind
      });
      if (created === null) return;
      if (created.hostKind !== hostKind || !this.isCurrent(epoch, source)) {
        await this.workspace.disposePreview(created.previewId);
        return;
      }
      this.publishSnapshot(epoch, source, created);
      const started = await this.startObservedPreview(epoch, source, created.previewId);
      if (started !== null && this.isCurrent(epoch, source)) this.publishSnapshot(epoch, source, started);
    } catch (error) {
      if (this.isCurrent(epoch, source)) this.publishTerminal(epoch, source, previewPhaseForBackendError(error), null);
    }
  }

  private async startCommittedPreview(
    epoch: number,
    source: PreviewSourceProjection,
    previewId: string
  ) {
    try {
      const started = await this.startObservedPreview(epoch, source, previewId);
      if (started !== null && this.isCurrent(epoch, source)) this.publishSnapshot(epoch, source, started);
    } catch (error) {
      if (this.isCurrent(epoch, source)) this.publishTerminal(epoch, source, previewPhaseForBackendError(error), null);
    }
  }

  private async switchAndStart(epoch: number, previewId: string, source: PreviewSourceProjection) {
    const requestId = this.requestId(epoch);
    try {
      const switched = await this.workspace.switchPreviewSource({
        previewId,
        requestId,
        source: source.previewSource
      });
      if (switched === null || !this.isCurrent(epoch, source)) return;
      this.publishSnapshot(epoch, source, switched);
      const started = await this.startObservedPreview(epoch, source, previewId);
      if (started !== null && this.isCurrent(epoch, source)) this.publishSnapshot(epoch, source, started);
    } catch (error) {
      if (this.isCurrent(epoch, source)) this.publishTerminal(epoch, source, previewPhaseForBackendError(error), null);
    }
  }

  private async startObservedPreview(
    epoch: number,
    source: PreviewSourceProjection,
    previewId: string
  ) {
    const startedPromise = this.workspace.startPreview(previewId);
    this.beginPreviewObservation(epoch, source, previewId);
    try {
      return await startedPromise;
    } finally {
      this.stopPreviewObservationIfCurrent(epoch, source, previewId);
    }
  }

  private beginPreviewObservation(
    epoch: number,
    source: PreviewSourceProjection,
    previewId: string
  ) {
    this.stopPreviewObservation();
    const observation: PendingPreviewSnapshotObservation = {
      epoch,
      source,
      previewId,
      requestCount: 0,
      inFlight: false,
      timer: null
    };
    this.previewObservationValue = observation;
    this.requestPreviewObservation(observation);
  }

  private requestPreviewObservation(observation: PendingPreviewSnapshotObservation) {
    if (!this.isPreviewObservationCurrent(observation) || observation.inFlight) return;
    if (observation.requestCount >= MAX_PREVIEW_SNAPSHOT_OBSERVATIONS) {
      this.stopPreviewObservationIfCurrent(observation.epoch, observation.source, observation.previewId);
      return;
    }
    observation.inFlight = true;
    observation.requestCount += 1;
    let continueObservation = false;
    void this.workspace.snapshotPreview(observation.previewId)
      .then((snapshot) => {
        if (!this.isPreviewObservationCurrent(observation)) return;
        if (snapshot === null || snapshot.previewId !== observation.previewId) {
          this.stopPreviewObservationIfCurrent(observation.epoch, observation.source, observation.previewId);
          return;
        }
        this.publishSnapshot(observation.epoch, observation.source, snapshot);
        if (!this.isPreviewObservationCurrent(observation)) return;
        continueObservation = snapshotNeedsObservation(snapshot);
        if (!continueObservation) {
          this.stopPreviewObservationIfCurrent(observation.epoch, observation.source, observation.previewId);
        }
      })
      .catch(() => {
        this.stopPreviewObservationIfCurrent(observation.epoch, observation.source, observation.previewId);
      })
      .finally(() => {
        observation.inFlight = false;
        if (continueObservation) this.schedulePreviewObservation(observation);
      });
  }

  private schedulePreviewObservation(observation: PendingPreviewSnapshotObservation) {
    if (!this.isPreviewObservationCurrent(observation)
      || observation.inFlight
      || observation.timer !== null
      || observation.requestCount >= MAX_PREVIEW_SNAPSHOT_OBSERVATIONS) return;
    observation.timer = setTimeout(() => {
      observation.timer = null;
      this.requestPreviewObservation(observation);
    }, PREVIEW_SNAPSHOT_OBSERVATION_INTERVAL_MS);
  }

  private stopPreviewObservation() {
    const observation = this.previewObservationValue;
    if (observation !== null && observation.timer !== null) clearTimeout(observation.timer);
    if (observation !== null) observation.timer = null;
    this.previewObservationValue = null;
  }

  private stopPreviewObservationIfCurrent(
    epoch: number,
    source: PreviewSourceProjection,
    previewId: string
  ) {
    const observation = this.previewObservationValue;
    if (observation === null
      || observation.epoch !== epoch
      || observation.previewId !== previewId
      || !sameSource(observation.source, source)) return;
    this.stopPreviewObservation();
  }

  private isPreviewObservationCurrent(observation: PendingPreviewSnapshotObservation) {
    return this.previewObservationValue === observation
      && this.stateValue.previewId === observation.previewId
      && this.isCurrent(observation.epoch, observation.source);
  }

  private publishSnapshot(epoch: number, source: PreviewSourceProjection, snapshot: PreviewSnapshot) {
    if (!this.isCurrent(epoch, source)) return;
    const phase = phaseForSnapshot(snapshot);
    this.stateValue = {
      ...this.stateValue,
      previewId: snapshot.previewId,
      snapshot,
      phase
    };
    this.emit();
  }

  private publishTerminal(
    epoch: number,
    source: PreviewSourceProjection,
    phase: PreviewExperiencePhase,
    snapshot: PreviewSnapshot | null
  ) {
    if (!this.isCurrent(epoch, source)) return;
    this.stateValue = { ...this.stateValue, snapshot, phase };
    this.emit();
  }

  private isCurrent(epoch: number, source: PreviewSourceProjection) {
    return this.stateValue.visible
      && this.stateValue.frontendEpoch === epoch
      && sameSource(this.stateValue.source, source);
  }

  private isCapturedFloating(captured: PreviewExperienceState) {
    return this.stateValue.visible
      && this.stateValue.host === "floating"
      && this.stateValue.frontendEpoch === captured.frontendEpoch
      && this.stateValue.previewId === captured.previewId
      && sameSource(this.stateValue.source, captured.source);
  }

  private requestId(epoch: number) {
    this.nextRequest += 1;
    return `w3-02-preview-${epoch}-${this.nextRequest}`;
  }

  private emit() {
    const state = this.stateWithNavigation();
    for (const listener of this.listeners) listener(state);
  }

  private stateWithNavigation(): PreviewExperienceState {
    return {
      ...this.stateValue,
      navigation: previewSiblingNavigationState(this.siblingNavigationValue, this.stateValue.source),
      navigationBusy: this.navigationBusyValue
    };
  }
}

export function isPreviewWorkspaceSpaceEligible(
  event: PreviewSpaceEvent
) {
  if (event.defaultPrevented === true || event.isComposing === true || event.altKey || event.repeat === true) return false;
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (target === null) return true;
  return target.closest(
    "input, textarea, select, [contenteditable='true'], [role='textbox'], [role='menu'], [role='dialog'], [aria-modal='true']"
  ) === null;
}

const FLOATING_PREVIEW_INTERACTIVE_SELECTOR = [
  "input",
  "textarea",
  "select",
  "[contenteditable='true']",
  "[role='textbox']",
  "button",
  "a[href]",
  "[role='button']",
  "[role='menuitem']",
  "[role='option']",
  "[role='checkbox']",
  "[role='radio']",
  "[role='switch']",
  "[role='tab']",
  "[role='link']"
].join(", ");

export function isFloatingPreviewCloseSpaceEligible(
  event: PreviewSpaceEvent
) {
  if (event.defaultPrevented === true || event.isComposing === true || event.altKey || event.repeat === true) return false;
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (target === null) return true;
  return target.closest(FLOATING_PREVIEW_INTERACTIVE_SELECTOR) === null;
}

export interface FloatingPreviewSpaceEvent extends PreviewSpaceEvent {
  readonly key: string;
  readonly preventDefault: () => void;
}

export function handleFloatingPreviewSpace(
  event: FloatingPreviewSpaceEvent,
  close: () => boolean
) {
  if (event.key !== " " && event.key !== "Space") return false;
  if (!isFloatingPreviewCloseSpaceEligible(event)) return false;
  if (!close()) return false;
  event.preventDefault();
  return true;
}

/**
 * Maps only stable backend error codes that already have a host-neutral
 * Preview phase. Unknown errors remain generic and are never rendered raw.
 */
export function previewPhaseForBackendError(error: unknown): PreviewExperiencePhase {
  const code = error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : "";
  switch (code) {
    case "preview_source_unavailable": return "source_unavailable";
    case "preview_materialization_required": return "materialization_required";
    case "preview_permission_denied": return "permission_denied";
    case "preview_source_identity_changed": return "identity_changed";
    case "preview_cancelled": return "cancelled";
    default: return "error";
  }
}

/**
 * Reduces backend provider warnings to a stable, non-technical fallback
 * state. Provider ids stay diagnostic data and are never required for the
 * user-facing decision.
 */
export function previewFallbackState(
  snapshot: PreviewSnapshot | null
): PreviewFallbackState | null {
  const envelope = snapshot?.representation;
  if (envelope?.representation.family !== "metadata") return null;

  const reasons = envelope.warnings
    .filter((warning): warning is Extract<PreviewWarning, { kind: "provider_fallback" }> => warning.kind === "provider_fallback")
    .map((warning) => warning.reason);
  if (reasons.some((reason) => reason === "failed" || reason === "timeout" || reason === "corrupt_source")) {
    return "failed";
  }
  if (reasons.some((reason) => reason === "unsupported")) return "unsupported";
  return "metadata";
}

/**
 * Projects the controller phase and backend completeness into the state
 * grammar consumed by both Floating and Pinned Preview shells.
 */
export function previewPresentationState(
  phase: PreviewExperiencePhase,
  snapshot: PreviewSnapshot | null = null,
  imagePresentationState: PreviewImagePresentationState | null = null,
  pdfPresentationState: PreviewImagePresentationState | null = null
): PreviewPresentationState {
  switch (phase) {
    case "closed": return "closed";
    case "no_source": return "no_source";
    case "resolving": return "resolving";
    case "loading": return "loading";
    case "content":
      return snapshot?.representation === undefined
        ? "failed"
        : snapshot.representation.representation.family === "image"
          ? imagePresentationState ?? "loading"
          : snapshot.representation.representation.family === "pdf"
            ? pdfPresentationState ?? "loading"
          : snapshot.representation.completeness === "complete" ? "ready" : "partial";
    case "metadata_fallback": {
      const fallback = previewFallbackState(snapshot);
      return fallback === "unsupported"
        ? "unsupported"
        : fallback === "failed"
          ? "failed"
          : "metadata_fallback";
    }
    case "unsupported_representation": return "unsupported";
    case "source_unavailable": return "unavailable";
    case "permission_denied": return "permission_required";
    case "materialization_required": return "materialization_required";
    case "identity_changed": return "identity_changed";
    case "cancelled": return "cancelled";
    case "error": return "failed";
  }
}

function sameSource(left: PreviewSourceProjection | null, right: PreviewSourceProjection | null) {
  return left?.key === right?.key;
}

function isRestorableFocusTarget(target: HTMLElement | null): target is HTMLElement {
  return Boolean(target?.isConnected
    && target !== document.body
    && target !== document.documentElement
    && !target.hasAttribute("disabled")
    && (target.tabIndex >= 0 || target.matches("button, input, select, textarea, a[href], [contenteditable='true']")));
}

function phaseForSnapshot(snapshot: PreviewSnapshot): PreviewExperiencePhase {
  if (snapshot.state === "idle" || snapshot.state === "resolving" || snapshot.state === "preparing") return "resolving";
  if (snapshot.state === "loading") return "loading";
  if (snapshot.state === "cancelled") return "cancelled";
  const terminalPhase = phaseForTerminalWarning(snapshot);
  if (terminalPhase !== null) return terminalPhase;
  if (snapshot.state === "failed" || snapshot.state === "disposed") return "error";

  const representation = snapshot.representation?.representation;
  if (representation === undefined) return "metadata_fallback";
  if (["text", "safe_html", "structured_tree", "table", "image", "pdf", "folder_summary", "archive_tree", "native_opaque"].includes(representation.family)) return "content";
  if (representation.family !== "metadata") return "unsupported_representation";
  return phaseForEligibility(representation.metadata.readEligibility);
}

function phaseForTerminalWarning(snapshot: PreviewSnapshot): PreviewExperiencePhase | null {
  const terminalWarning = snapshot.representation?.warnings.find((warning) => warning.kind === "terminal_condition");
  if (terminalWarning === undefined) return null;
  return phaseForTerminalCondition(terminalWarning.condition);
}

function phaseForTerminalCondition(condition: PreviewTerminalCondition): PreviewExperiencePhase {
  switch (condition) {
    case "source_unavailable": return "source_unavailable";
    case "materialization_required": return "materialization_required";
    case "permission_denied": return "permission_denied";
    case "identity_changed": return "identity_changed";
    case "cancelled": return "cancelled";
  }
}

function snapshotNeedsObservation(snapshot: PreviewSnapshot) {
  return snapshot.representation?.completeness === "partial"
    || snapshot.state === "resolving"
    || snapshot.state === "preparing"
    || snapshot.state === "loading";
}

function phaseForEligibility(eligibility: ContentReadEligibility): PreviewExperiencePhase {
  switch (eligibility) {
    case "materialization_required":
    case "downloading":
    case "metadata_only":
      return eligibility === "materialization_required" || eligibility === "downloading"
        ? "materialization_required"
        : "metadata_fallback";
    case "permission_required":
      return "permission_denied";
    case "identity_changed":
      return "identity_changed";
    case "source_unavailable":
    case "source_not_supported":
    case "package_unsupported":
    case "symlink":
    case "availability_unknown":
      return "source_unavailable";
    case "eligible":
      return "metadata_fallback";
    default:
      return "error";
  }
}
