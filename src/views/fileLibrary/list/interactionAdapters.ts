import type { BrowsePresentationEntry, LibraryPresentationEntry } from "../presentation/contracts";
import { libraryPresentationEntryAt } from "../library/librarySourceOwner";
import type { LibrarySourceOwner } from "../library/librarySourceOwner";
import type { BrowseSourceOwner } from "../browse/browseSourceOwner";
import type {
  BrowseInteractionProjection,
  LibraryInteractionProjection,
  PresentationInteractionProjection,
  PresentationSelectionIntent
} from "./interactionContracts";

type LibraryInteractionSource = Pick<
  LibrarySourceOwner,
  | "files"
  | "totalCount"
  | "collection"
  | "focusedId"
  | "selection"
  | "selectionContainsFileId"
  | "hasMore"
  | "isLoading"
  | "loadNextPage"
  | "setExplicitSelection"
  | "setFocusedId"
  | "toggleSelection"
  | "selectAllMatching"
  | "clearSelection"
>;

type BrowseInteractionSource = Pick<
  BrowseSourceOwner,
  | "entries"
  | "collection"
  | "focusedId"
  | "selectedIds"
  | "hasMore"
  | "enumerationState"
  | "loadNextPage"
  | "selectEntry"
  | "selectAllDirectory"
  | "selectIds"
  | "invertSelection"
  | "clearSelection"
  | "setFocusedId"
>;

/**
 * Creates a component-facing Library projection. It delegates every state
 * transition to the existing Query V2 and LibrarySelectionV1 owner.
 */
export function createLibraryInteractionProjection(
  source: LibraryInteractionSource
): LibraryInteractionProjection {
  const loadedIds = source.files.map((file) => file.id);
  const focusedIndex = source.files.findIndex((file) => file.id === source.focusedId);
  const rowCount = Math.max(source.files.length, source.totalCount ?? source.files.length);

  const select = (entry: LibraryPresentationEntry, index: number, intent: PresentationSelectionIntent) => {
    const fileId = entry.entryRef.fileId;
    if (intent === "replace") {
      source.setExplicitSelection([fileId], fileId, index);
      return;
    }
    source.toggleSelection(fileId, loadedIds, intent === "range");
  };
  const focus = (entry: LibraryPresentationEntry, index: number) => {
    source.setFocusedId(entry.entryRef.fileId, index);
  };

  return {
    source: "library",
    collection: source.collection,
    rowCount,
    loadedRowCount: source.files.length,
    entryAt: (index) => libraryPresentationEntryAt(source.files, index),
    focusedIndex,
    isSelected: (entry) => source.selectionContainsFileId(entry.entryRef.fileId),
    isFocused: (entry) => entry.entryRef.fileId === source.focusedId,
    hasMore: source.hasMore,
    isLoadingMore: source.isLoading,
    selection: source.selection,
    capabilities: {
      selectAll: "all_matching",
      canActivate: true,
      canLoadMore: source.hasMore
    },
    actions: {
      select,
      selectAll: source.selectAllMatching,
      clearSelection: source.clearSelection,
      focus,
      loadMore: source.loadNextPage
    }
  };
}

/**
 * Creates a component-facing Browse projection. Browse Ctrl/Cmd+A selects the
 * current query-matching directory scope through the source owner.
 */
export function createBrowseInteractionProjection(
  source: BrowseInteractionSource
): BrowseInteractionProjection {
  const focusedIndex = source.focusedId === null
    ? -1
    : source.entries.findIndex((entry) => entry.entryRef.entryId === source.focusedId);
  const rowCount = source.collection?.provenance.completion === "complete"
    && source.collection.provenance.knownCount !== undefined
    ? Math.max(source.entries.length, source.collection.provenance.knownCount)
    : source.entries.length;

  const select = (entry: BrowsePresentationEntry, _index: number, intent: PresentationSelectionIntent) => {
    source.selectEntry(entry.entryRef.entryId, intent);
  };
  const focus = (entry: BrowsePresentationEntry) => {
    source.setFocusedId(entry.entryRef.entryId);
  };

  return {
    source: "browse",
    collection: source.collection,
    rowCount,
    loadedRowCount: source.entries.length,
    entryAt: (index) => source.entries[index],
    focusedIndex,
    isSelected: (entry) => source.selectedIds.has(entry.entryRef.entryId),
    isFocused: (entry) => entry.entryRef.entryId === source.focusedId,
    hasMore: source.hasMore,
    isLoadingMore: source.enumerationState === "loading_more",
    selection: source.selectedIds,
    capabilities: {
      selectAll: "all_matching",
      canActivate: true,
      canLoadMore: source.hasMore
    },
    actions: {
      select,
      selectAll: source.selectAllDirectory,
      clearSelection: source.clearSelection,
      focus,
      loadMore: source.loadNextPage,
      selectIds: (ids, additive) => source.selectIds(ids, additive ? "add" : "replace"),
      invertSelection: source.invertSelection
    }
  };
}

export function isLibraryInteraction(
  interaction: PresentationInteractionProjection
): interaction is LibraryInteractionProjection {
  return interaction.source === "library";
}

export function isBrowseInteraction(
  interaction: PresentationInteractionProjection
): interaction is BrowseInteractionProjection {
  return interaction.source === "browse";
}

export function selectionIntentFromModifiers(event: Pick<MouseEvent, "shiftKey" | "metaKey" | "ctrlKey">): PresentationSelectionIntent {
  if (event.shiftKey) return "range";
  if (event.metaKey || event.ctrlKey) return "toggle";
  return "replace";
}
