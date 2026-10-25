import { tauriApi } from "../../api/tauriApi";
import { samePathLike } from "../../utils/viewHelpers";
import { explicitSingleSelectionId, useFileLibraryInspectorStore, useFileLibrarySelectionStore } from "../useFileLibraryV2Store";

/** Match journal Win32 extended paths to the existing library path-ID format.
 * This is UI projection only, never filesystem execution authority. */
export function libraryPathFromJournal(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  if (/^\/\/\?\/UNC\//i.test(normalized)) return `//${normalized.slice(8)}`;
  if (/^\/\/\?\/[a-z]:\//i.test(normalized)) return normalized.slice(4);
  return normalized;
}

export function sameJournalLibraryPath(left: string, right: string): boolean {
  return samePathLike(libraryPathFromJournal(left), libraryPathFromJournal(right));
}

/** UI projection only. A log path is a legacy-ID lookup candidate, never an
 * executable identity: accept only the canonical ID/path returned by backend. */
export async function refreshRenamedSelection(previousId: string, committedPath: string) {
  committedPath = libraryPathFromJournal(committedPath);
  const originalSelection = useFileLibrarySelectionStore.getState().selection;
  if (explicitSingleSelectionId(originalSelection) !== previousId) return;
  for (const candidate of new Set([previousId, committedPath])) {
    try {
      const detail = await tauriApi.getFileLibraryDetail(candidate);
      if (detail.isStale || !sameJournalLibraryPath(detail.path, committedPath)) continue;
      const current = useFileLibrarySelectionStore.getState();
      if (current.selection !== originalSelection) return;
      current.setExplicit([detail.id], current.focusedId === previousId ? detail.id : current.focusedId, -1);
      if (detail.id !== previousId) {
        if (useFileLibraryInspectorStore.getState().selectedId === previousId) useFileLibraryInspectorStore.getState().clear();
      }
      return;
    } catch {
      // Old path IDs may have been retired by the existing index publication.
    }
  }
  const current = useFileLibrarySelectionStore.getState();
  if (current.selection === originalSelection) {
    current.clear();
    if (current.focusedId && current.focusedId !== previousId) current.setFocused(current.focusedId, current.anchorIndex);
    if (useFileLibraryInspectorStore.getState().selectedId === previousId) useFileLibraryInspectorStore.getState().clear();
  }
}
