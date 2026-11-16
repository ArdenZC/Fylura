import type { LibrarySelectionV1 } from "../../../types/domain";
import type {
  BrowsePresentationCollectionContext,
  BrowsePresentationEntry,
  LibraryPresentationCollectionContext,
  LibraryPresentationEntry,
  PresentationEntry
} from "../presentation/contracts";

export type PresentationSelectionIntent = "replace" | "toggle" | "range";

export interface InteractionCapabilities {
  /** Ctrl/Cmd+A remains a source-owned operation, never a rendered-row loop. */
  readonly selectAll: "all_matching" | "loaded";
  readonly canActivate: boolean;
  readonly canLoadMore: boolean;
}

export interface InteractionActions<Entry extends PresentationEntry> {
  readonly select: (entry: Entry, index: number, intent: PresentationSelectionIntent) => void;
  readonly selectAll: () => void;
  readonly clearSelection: () => void;
  readonly focus: (entry: Entry, index: number) => void;
  readonly loadMore: () => void | Promise<void>;
  /** Browse-only rectangular selection over the currently mounted rows/cells. */
  readonly selectIds?: (ids: readonly string[], additive: boolean) => void;
  readonly invertSelection?: () => void;
}

interface InteractionProjectionBase<Entry extends PresentationEntry, Collection> {
  readonly collection: Collection | null;
  /** Exact logical count when the source has published one; otherwise loaded count. */
  readonly rowCount: number;
  /** Number of entries that can currently be adapted by entryAt. */
  readonly loadedRowCount: number;
  readonly entryAt: (index: number) => Entry | undefined;
  /** Focus is source state, not a mounted DOM row. */
  readonly focusedIndex: number;
  /** Membership is deliberately bound to the current source and entry. */
  readonly isSelected: (entry: Entry) => boolean;
  readonly isFocused: (entry: Entry) => boolean;
  readonly hasMore: boolean;
  readonly isLoadingMore: boolean;
  readonly capabilities: InteractionCapabilities;
  readonly actions: InteractionActions<Entry>;
}

export interface LibraryInteractionProjection
  extends InteractionProjectionBase<LibraryPresentationEntry, LibraryPresentationCollectionContext> {
  readonly source: "library";
  readonly selection: LibrarySelectionV1 | null;
}

export interface BrowseInteractionProjection
  extends InteractionProjectionBase<BrowsePresentationEntry, BrowsePresentationCollectionContext> {
  readonly source: "browse";
  readonly selection: ReadonlySet<string>;
}

export type PresentationInteractionProjection =
  | LibraryInteractionProjection
  | BrowseInteractionProjection;
