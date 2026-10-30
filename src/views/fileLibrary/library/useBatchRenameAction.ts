import { useCallback, useMemo } from "react";
import type { LibrarySourceOwner } from "./librarySourceOwner";
import type { OperationPreview } from "../../../types/domain";
import type { View } from "../../../types/ui";

export function useBatchRenameAction(source: LibrarySourceOwner, setView: (view: View) => void) {
  const actionSource = useMemo(() => ({
    selection: source.selection,
    selectedFiles: source.selectedFiles,
    selectedIdList: source.selectedIdList,
    selectionSummary: source.selectionSummary
  }), [source.selection, source.selectedFiles, source.selectedIdList, source.selectionSummary]);
  const onPreview = useCallback((previews: OperationPreview[]) => {
    source.clearExecutionIntent();
    source.setPreviewResult({ previews, total: previews.length, limit: previews.length, offset: 0, truncated: false, hasMore: false }, source.scope);
    setView("preview");
  }, [setView, source.clearExecutionIntent, source.scope, source.setPreviewResult]);
  return { source: actionSource, onPreview };
}
