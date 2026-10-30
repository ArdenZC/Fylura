import { useEffect, useRef, useState } from "react";
import { ListOrdered } from "lucide-react";
import type { OperationPreview } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonSubtle, cn } from "../../../utils/tw";
import { BatchRenameDialog } from "./BatchRenameDialog";
import { BATCH_RENAME_LIMIT, isBatchRenameSelectionEligible } from "./batchRenameModel";
import type { LibrarySourceOwner } from "./librarySourceOwner";

type Props = {
  source: Pick<LibrarySourceOwner, "selection" | "selectedFiles" | "selectedIdList" | "selectionSummary">;
  renameAvailable: boolean;
  paneVisible: boolean;
  t: Translator;
  onPreview: (previews: OperationPreview[]) => void;
};

export function BatchRenameAction({ source, renameAvailable, paneVisible, t, onPreview }: Props) {
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const openedSelectionKey = useRef("");
  const [open, setOpen] = useState(false);
  const files = source.selection?.kind === "explicit" ? source.selectedFiles : [];
  const visible = renameAvailable && (source.selection?.kind === "all_matching" || source.selectedIdList.length > 1);
  const ready = isBatchRenameSelectionEligible(source.selection?.kind, source.selectedIdList, files, renameAvailable);
  const disabled = !ready;
  const selectionKey = `${source.selection?.kind ?? "none"}:${source.selectedIdList.join("|")}`;
  const showDialog = open && paneVisible && ready && openedSelectionKey.current === selectionKey;

  useEffect(() => {
    if (open && !showDialog) setOpen(false);
  }, [open, showDialog]);

  function openDialog() {
    if (!ready) return;
    openedSelectionKey.current = selectionKey;
    setOpen(true);
  }

  function continueToPreview(previews: OperationPreview[]) {
    setOpen(false);
    onPreview(previews);
  }

  return <>
    {visible ? <button ref={triggerRef} className={cn(buttonSubtle, "min-h-9 px-3 py-1.5 text-xs")} disabled={disabled} title={source.selectedIdList.length > BATCH_RENAME_LIMIT ? t("libraryBatchRenameLimit").replace("{limit}", String(BATCH_RENAME_LIMIT)) : disabled ? t("libraryBatchRenameSelectionUnavailable") : undefined} onClick={openDialog}><ListOrdered size={15}/>{t("libraryBatchRename")} · {source.selection?.kind === "all_matching" ? source.selectionSummary?.count ?? "…" : source.selectedIdList.length}</button> : null}
    {showDialog ? <BatchRenameDialog
      key={`batch-rename-${files.map((file) => file.id).join("|")}`}
      files={files}
      t={t}
      onClose={() => setOpen(false)}
      restoreFocus={() => triggerRef.current}
      onPreview={continueToPreview}
    /> : null}
  </>;
}
