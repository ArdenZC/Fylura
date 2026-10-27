/**
 * Shared File Library 2.0 / Preview Platform wire contracts.
 *
 * These are serialized references and projections only. Authority-bearing
 * opaque refs and leases never contain filesystem paths or provide authority
 * to resolve, read, materialize, query, watch or mutate anything. Restore
 * locators may retain path-like routing hints as non-authoritative metadata.
 */

export type BrowseEntryRef = {
  kind: "ephemeral";
  /** Session-scoped, non-durable identity published by Browse only. */
  browseSessionId: string;
  entryId: string;
};

export type EntryRef =
  | {
      kind: "managed";
      /** Existing managed File Library identity. */
      fileId: string;
    }
  | BrowseEntryRef;

export type LocationRef =
  | {
      kind: "managed";
      /** Existing managed scan-root identity. */
      scanRootId: string;
    }
  | {
      kind: "ephemeral";
      /** Session-scoped, non-durable identity. */
      browseSessionId: string;
      locationId: string;
    };

/** Opaque session-scoped path reference; never a filesystem path. */
export interface BrowsePathRef {
  id: string;
}

export type LibraryNavigationSource =
  | "smart_view"
  | "saved_view"
  | "tag"
  | "search"
  | "custom";

export type NavigationTarget =
  | {
      kind: "library";
      source: LibraryNavigationSource;
      key: string;
    }
  | {
      kind: "browse";
      location: LocationRef;
      pathRef: BrowsePathRef;
    };

export interface BrowseEnumerationRef {
  sessionId: string;
  requestId: string;
  enumerationId: string;
}

export type WorkspacePlatform = "macos" | "windows";

/** Persistent routing/presentation metadata, never a live workspace authority. */
export type WorkspaceRestoreLocator =
  | {
      kind: "library";
      source: LibraryNavigationSource;
      key: string;
    }
  | {
      kind: "browse";
      platform: WorkspacePlatform;
      routingHint: string;
      displayHint?: string;
    };

export type LocationKind = "local" | "external" | "network" | "cloud_provider" | "unknown";

export type LocationAvailability =
  | "available"
  | "offline"
  | "disconnected"
  | "permission_denied"
  | "authentication_required"
  | "not_found"
  | "unknown";

export type LocationFreshness = "current" | "stale" | "reconciling" | "unknown" | "not_applicable";

/** Entry/source-scoped content/materialization projection, not read authority. */
export type MaterializationState =
  | "local"
  | "boundary_readable"
  | "metadata_only"
  | "remote_placeholder"
  | "hydrating"
  | "unavailable"
  | "unknown";

/** Projection over the existing authoritative byte-read/open boundary. */
export type ContentReadEligibility =
  | "eligible"
  | "materialization_required"
  | "downloading"
  | "metadata_only"
  | "permission_required"
  | "source_unavailable"
  | "source_not_supported"
  | "package_unsupported"
  | "symlink"
  | "identity_changed"
  | "availability_unknown";

export type WorkClass = "foreground" | "interactive" | "background";

export type PreviewSourceRef =
  | {
      kind: "managed";
      fileId: string;
    }
  | {
      kind: "ephemeral";
      browseSessionId: string;
      entryId: string;
    }
  | {
      kind: "host_provided";
      hostToken: string;
    };

export type PreviewHostKind =
  | "zen_floating"
  | "zen_pinned"
  | "mac_quick_look_extension"
  | "windows_quick_preview"
  | "windows_preview_handler";

/** Opaque request/source-version-bound content access handle; never a path. */
export interface ContentReadLeaseRef {
  leaseId: string;
  requestId: string;
  sourceVersion: string;
}

export interface LocationCapabilities {
  canBrowse: boolean;
  canReadMetadata: boolean;
  canPreview: boolean;
  canWatch: boolean;
  canRequestMaterialization: boolean;
  canAddToLibrary: boolean;
}

/**
 * Non-authoritative Location projection. It contains only coarse runtime
 * capabilities; materialization and byte-read eligibility remain
 * entry/source-scoped projections.
 */
export interface LocationDescriptor {
  ref: LocationRef;
  displayName: string;
  kind: LocationKind;
  availability: LocationAvailability;
  freshness: LocationFreshness;
  capabilities: LocationCapabilities;
}

export interface BrowseOpenRequest {
  platform: WorkspacePlatform;
  /** Admission/routing input only; never a live NavigationTarget authority. */
  routingHint: string;
  displayHint?: string;
  /** ID-only search intent; backend ignores renderer routing/display hints. */
  globalEntryId?: string;
}

export interface BrowseOpenResponse {
  sessionId: string;
  location: LocationDescriptor;
  rootPathRef: BrowsePathRef;
  /** Bounded, freshly enumerated Browse authority from search admission. */
  initialPage?: BrowsePage;
  focusEntryRef?: BrowseEntryRef;
}

export interface BrowseRestoreRequest {
  locator: WorkspaceRestoreLocator;
}

/** Backend-owned Location -> Browse action input. No renderer routing or path fields. */
export interface LocationBrowseRequest {
  location: LocationRef;
}

export type BrowseQueryEntryKind = "all" | "file" | "directory";

export interface BrowseSortSpec {
  field: "name" | "type" | "size" | "modified";
  direction: "asc" | "desc";
  foldersFirst: boolean;
}

export interface BrowseQuerySpecV1 {
  sort?: BrowseSortSpec;
  text: string | null;
  entryKind: BrowseQueryEntryKind;
}

export interface BrowseStartEnumerationRequest {
  sessionId: string;
  requestId: string;
  pathRef: BrowsePathRef;
  pageSize: number;
  query: BrowseQuerySpecV1;
}

export interface BrowseNextPageRequest {
  sessionId: string;
  cursor: string;
  pageSize: number;
}

export type BrowseCancelRequest =
  | {
    sessionId: string;
    enumeration: BrowseEnumerationRef;
    requestId?: never;
  }
  | {
    sessionId: string;
    enumeration?: never;
    requestId: string;
  };

export interface BrowseReleasePageRequest {
  page: BrowsePage;
}

export interface BrowseReleasePathRequest {
  sessionId: string;
  pathRef: BrowsePathRef;
}

export interface BrowseRetainPathRequest {
  sessionId: string;
  pathRef: BrowsePathRef;
}

export interface BrowseSessionRequest {
  sessionId: string;
}

export type BrowseCompletion = "partial" | "complete";
export type BrowseEntryKind = "file" | "directory";

export interface BrowseEntry {
  /** Browse can publish only its session-scoped ephemeral identity. */
  ref: BrowseEntryRef;
  pathRef?: BrowsePathRef;
  name: string;
  /** Presentation only; never send this value back as a resolver input. */
  displayPath: string;
  kind: BrowseEntryKind;
  extension?: string;
  size?: number;
  modifiedAt?: number;
  createdAt?: number;
  materialization: MaterializationState;
}

export interface BrowsePage {
  skippedEntries?: number;
  sessionId: string;
  requestId: string;
  enumerationId: string;
  entries: BrowseEntry[];
  nextCursor?: string;
  completion: BrowseCompletion;
  knownCount?: number;
}

export interface ChangeStartRequest {
  sessionId: string;
  pathRef: BrowsePathRef;
}

export interface ChangeStartResponse {
  monitorId: string;
  sessionId: string;
  pathRef: BrowsePathRef;
}

export interface ChangePendingRequest {
  monitorId: string;
}

export type ChangeKind = "content_changed" | "renamed" | "target_unavailable" | "uncertain";

export interface ChangeHint {
  kind: ChangeKind;
}

export interface ChangePendingResponse {
  monitorId: string;
  sequence: number;
  hint: ChangeHint;
}

export interface ChangeRefreshRequest {
  monitorId: string;
  requestId: string;
  pageSize: number;
  query: BrowseQuerySpecV1;
}

export interface ReadEligibilityRequest {
  source: PreviewSourceRef;
}

export interface ReadEligibilityResponse {
  source: PreviewSourceRef;
  eligibility: ContentReadEligibility;
}

export type ThumbnailVariant = "small" | "medium" | "large";

export interface ThumbnailRequest {
  requestId: string;
  source: EntryRef;
  variant: ThumbnailVariant;
  workClass: WorkClass;
  sessionId?: string;
}

export interface ThumbnailCancelRequest {
  requestId: string;
}

export interface ThumbnailArtifact {
  /** Logical cache identity only; never a staging/cache filesystem path. */
  cacheKey: string;
  /** Binary IPC payload decoded into an owned typed array. */
  bytes: Uint8Array;
}

export interface PreviewAssetArtifact {
  mediaType: string;
  bytes: Uint8Array;
}

export interface PreviewCreateRequest {
  requestId: string;
  source: PreviewSourceRef;
  hostKind: PreviewHostKind;
}

export interface PreviewSessionRequest {
  previewId: string;
  nativePresentation?: PreviewNativePresentation;
}

export interface PreviewNativePresentation {
  host: PreviewHostKind;
  token: string;
  sourceVersion: string;
  bounds: PreviewNativeBounds;
}

export interface PreviewNativeBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PreviewSwitchSourceRequest {
  previewId: string;
  requestId: string;
  source: PreviewSourceRef;
}

/** Exact Preview-session/request/sourceVersion-bound opaque asset request. */
export interface PreviewAssetRequest {
  previewId: string;
  requestId: string;
  sourceVersion: string;
  assetToken: string;
  /** Range requests are used only by range-backed local PDF previews. */
  offsetBytes?: number;
  maxBytes?: number;
}

export type PreviewSessionState =
  | "idle"
  | "resolving"
  | "preparing"
  | "loading"
  | "ready"
  | "failed"
  | "cancelled"
  | "disposed";

export interface PreviewCapabilities {
  canSearch: boolean;
  canZoom: boolean;
  canPlayback: boolean;
  canSelectText: boolean;
  canNavigateInternal: boolean;
  canNavigateSiblings: boolean;
  canOpenExternal: boolean;
  canReveal: boolean;
  canRequestMaterialization: boolean;
}

export interface PreviewMetadata {
  displayName: string;
  mediaType: string | null;
  extension: string | null;
  sizeBytes: number | null;
  modifiedAtEpochMs: number | null;
  materialization: MaterializationState;
  readEligibility: ContentReadEligibility;
}

export type PreviewRepresentation =
  | {
      family: "metadata";
      metadata: PreviewMetadata;
    }
  | {
      family: "text";
      text: string;
      language: string | null;
    }
  | {
      family: "safe_html";
      html: string;
    }
  | {
      family: "structured_tree";
      encodedTree: string;
    }
  | {
      family: "table";
      encodedTable: string;
    }
  | {
      family: "image";
      assetToken: string;
      mediaType: string;
    }
  | {
      family: "pdf";
      assetToken: string;
      mediaType: string;
      /** Authoritative logical PDF length used by PDFDataRangeTransport. */
      lengthBytes: number;
    }
  | {
      family: "media";
      assetToken: string;
      mediaType: string;
    }
  | {
      family: "folder_summary";
      encodedSummary: string;
    }
  | {
      family: "archive_tree";
      encodedTree: string;
    }
  | {
      family: "native_opaque";
      host: PreviewHostKind;
      token: string;
    };

export type PreviewProviderErrorCode =
  | "unsupported"
  | "failed"
  | "timeout"
  | "corrupt_source"
  | "source_unavailable"
  | "materialization_required"
  | "permission_denied"
  | "identity_changed"
  | "cancelled";

export type PreviewRecoverableProviderErrorCode =
  | "unsupported"
  | "failed"
  | "timeout"
  | "corrupt_source";

export type PreviewTerminalCondition =
  | "source_unavailable"
  | "materialization_required"
  | "permission_denied"
  | "identity_changed"
  | "cancelled";

export type PreviewWarning =
  | {
      kind: "provider_fallback";
      providerId: string;
      reason: PreviewRecoverableProviderErrorCode;
    }
  | {
      kind: "metadata_fallback";
    }
  | {
      kind: "terminal_condition";
      condition: PreviewTerminalCondition;
    };

export interface PreviewRepresentationEnvelope {
  sourceVersion: string;
  representation: PreviewRepresentation;
  completeness: "complete" | "partial" | "unknown";
  warnings: PreviewWarning[];
  capabilities: PreviewCapabilities;
}

export interface PreviewSnapshot {
  previewId: string;
  sessionId: string;
  requestId: string;
  source: PreviewSourceRef;
  hostKind: PreviewHostKind;
  state: PreviewSessionState;
  sourceVersion?: string;
  representation?: PreviewRepresentationEnvelope;
  effectiveCapabilities: PreviewCapabilities;
  activeProviderId?: string;
}
