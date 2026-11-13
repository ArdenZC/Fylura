import type {
  FileQuerySpecV2
} from "../../../types/domain";
import type {
  BrowseCompletion,
  BrowseEntryRef,
  BrowsePathRef,
  EntryRef,
  MaterializationState
} from "../../../types/fileWorkspace";

declare const presentationRenderKeyBrand: unique symbol;

/** UI-only identity for a mounted presentation row. It has no resolver. */
export type PresentationRenderKey = string & {
  readonly [presentationRenderKeyBrand]: true;
};

export type PresentationEntryKind = "file" | "directory";

export type LibraryPresentationEntryRef = Extract<EntryRef, { kind: "managed" }>;
export type BrowsePresentationEntryRef = BrowseEntryRef;
export type LibraryPresentationAvailability = "available" | "missing";

interface PresentationEntryMetadata {
  renderKey: PresentationRenderKey;
  displayName: string;
  entryKind: PresentationEntryKind;
  extension?: string;
  typeHint?: string;
  size?: number;
  modifiedAt?: number;
  createdAt?: number;
  isHidden?: boolean;
  materialization?: MaterializationState;
}

export interface LibraryPresentationEntry extends PresentationEntryMetadata {
  source: "library";
  /** Existing managed identity; never replace this with renderKey. */
  entryRef: LibraryPresentationEntryRef;
  /** Bounded presentation of the current managed summary's availability fact. */
  availability: LibraryPresentationAvailability;
}

export interface BrowsePresentationEntry extends PresentationEntryMetadata {
  source: "browse";
  /** Session-scoped Browse identity retained without interpretation. */
  entryRef: BrowsePresentationEntryRef;
  /** Opaque folder-navigation reference; never a filesystem path. */
  pathRef?: BrowsePathRef;
  /** Presentation only; never accepted by a resolver. */
  displayPath?: string;
}

export type PresentationEntry = LibraryPresentationEntry | BrowsePresentationEntry;

export interface LibraryPresentationCollectionProvenance {
  queryFingerprint: string;
  snapshotRevision: number;
  /**
   * FileQueryResponseV2 does not repeat its request spec. When the source
   * owner has the canonical spec, it is retained once here for later source
   * ownership; it is never copied into entries.
   */
  query?: FileQuerySpecV2;
}

export interface LibraryPresentationCollectionContext {
  source: "library";
  provenance: LibraryPresentationCollectionProvenance;
}

export interface BrowsePresentationCollectionProvenance {
  sessionId: string;
  requestId: string;
  enumerationId: string;
  completion: BrowseCompletion;
  /** Published only when the Browse source says the count is exact. */
  knownCount?: number;
}

export interface BrowsePresentationCollectionContext {
  source: "browse";
  provenance: BrowsePresentationCollectionProvenance;
}

export type PresentationCollectionContext =
  | LibraryPresentationCollectionContext
  | BrowsePresentationCollectionContext;
