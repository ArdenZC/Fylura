import type { LibrarySourceOwner } from "../library/librarySourceOwner";
import type { BrowseSourceOwner } from "../browse/browseSourceOwner";
import { useFileManagementSurface } from "./fileManagementStore";

/** Adapt the existing source owners; selection and refresh stay with their source. */
export function useLibraryManagementSurface(source: LibrarySourceOwner) {
  useFileManagementSurface({
    signature: JSON.stringify(["library", source.selection, source.querySpecSignature]),
    count: source.selection?.kind === "all_matching" ? source.totalCount : source.selectedIdList.length,
    hasDirectorySelection: source.selectedFiles.some((file) => file.isDirectory),
    selection: source.selection ?? undefined,
    refresh: source.refreshResults,
    clear: source.clearSelection,
    selectAll: source.selectAllMatching,
    selectionScope: "all"
  });
}

export function useBrowseManagementSurface(source: BrowseSourceOwner) {
  useFileManagementSurface({
    signature: JSON.stringify(["browse", source.sessionId, source.currentPathRef?.id, source.query, source.selectedEntryRefs, source.directorySelection?.snapshotId, source.directorySelection?.revision, source.collection?.provenance.enumerationId, source.entries.length, source.enumerationState, source.pendingChange, source.changeState]),
    browseSource: source,
    directorySelection: source.directorySelection,
    count: source.selectedCount,
    hasDirectorySelection: Math.max(0, (source.directorySelection?.directoryCount ?? 0) - source.entries.filter(
      (entry) => entry.entryKind === "directory" && source.directorySelection?.excludedEntryIds.has(entry.entryRef.entryId),
    ).length) > 0 || source.entries.some(
      (entry) => source.selectedIds.has(entry.entryRef.entryId) && entry.entryKind === "directory",
    ),
    refs: [...source.selectedEntryRefs],
    browse: source.sessionId && source.currentPathRef
      ? { sessionId: source.sessionId, pathRef: source.currentPathRef } : undefined,
    refresh: source.refreshEnumeration,
    clear: source.clearSelection,
    selectAll: source.selectAllDirectory,
    selectionScope: "directory"
  });
}
