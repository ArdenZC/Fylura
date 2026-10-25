import { useEffect, useRef, useState, type FormEvent } from "react";
import { tauriApi } from "../../../api/tauriApi";
import { open } from "@tauri-apps/plugin-dialog";
import { ModalPortal } from "../../../components/modal/ModalPortal";
import type { FileLibrarySummary, OperationPreview } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonPrimary, buttonSecondary, cn, inputSurface, raisedSurface } from "../../../utils/tw";
import { localizedStableError, readableError } from "../../../utils/viewHelpers";

export function ManualRenameDialog({ file, t, onClose, onPreview, restoreFocus, mode = "rename", initialDirectory = "", initialName }: {
  file: Pick<FileLibrarySummary, "id" | "name">;
  mode?: "rename" | "move";
  initialDirectory?: string;
  initialName?: string;
  t: Translator;
  onClose: () => void;
  onPreview: (preview: OperationPreview) => void;
  restoreFocus: () => HTMLElement | null;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const active = useRef(true);
  const [name, setName] = useState(initialName ?? file.name);
  const [directory, setDirectory] = useState(initialDirectory);
  const [blocked, setBlocked] = useState<OperationPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    active.current = true;
    const value = inputRef.current?.value ?? file.name;
    const dot = value.lastIndexOf(".");
    inputRef.current?.setSelectionRange(0, dot > 0 ? dot : value.length);
    return () => { active.current = false; };
  }, [file.name]);

  function close() {
    active.current = false;
    onClose();
  }

  async function chooseDirectory() {
    if (busy) return;
    setBusy(true);
    try {
      const selected = await open({ directory: true, multiple: false });
      if (active.current && typeof selected === "string") { setDirectory(selected); setBlocked(null); setError(""); }
    } catch (failure) {
      if (active.current) setError(localizedStableError(failure, t));
    } finally { if (active.current) setBusy(false); }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !name.trim() || (mode === "rename" ? name.trim() === file.name : !directory)) return;
    setBusy(true);
    setError("");
    setBlocked(null);
    try {
      const preview = mode === "move"
        ? await tauriApi.getManualMoveOperationPreview(file.id, directory, name)
        : await tauriApi.getManualRenameOperationPreview(file.id, name);
      if (active.current) {
        if (preview.is_executable === false || preview.blocking_reason) setBlocked(preview);
        else onPreview(preview);
      }
    } catch (failure) {
      if (!active.current) return;
      const message = readableError(failure);
      setError(message.includes("manual_move_same_volume_required") || message.includes("manual_move_local_same_volume_required") ? t("libraryMoveSameVolume")
        : message.includes("manual_move_different_directory_required") ? t("libraryMoveDifferentDirectory")
        : /manual_move_destination_invalid|reparse_point|unsafe_path/.test(message) ? t("libraryMoveDestinationUnavailable")
        : message.includes("manual_move_source_unavailable") ? t("libraryRenameUnavailable")
        : message.includes("manual_rename_source_read_only") ? t("libraryRenameReadOnly")
        : message.includes("manual_rename_name_unchanged") ? t("libraryRenameUnchanged")
        : message.includes("manual_rename_source_unavailable") ? t("libraryRenameUnavailable") : localizedStableError(failure, t));
    } finally {
      if (active.current) setBusy(false);
    }
  }

  return <ModalPortal initialFocusRef={inputRef} restoreFocus={restoreFocus} onEscape={close}>
    <div className="fixed inset-0 z-50 grid place-items-center overflow-y-auto bg-[var(--zc-overlay)] p-4">
      <form onSubmit={(event) => void submit(event)} aria-busy={busy} role="dialog" aria-modal="true"
        aria-labelledby="manual-rename-title" aria-describedby="manual-rename-description"
        className={cn(raisedSurface, "grid max-h-[calc(100vh-2rem)] w-full max-w-lg gap-4 overflow-y-auto overscroll-contain p-5")}>
        <h2 id="manual-rename-title" className="text-lg font-semibold">{t(mode === "move" ? "libraryManualMove" : "libraryManualRename")}</h2>
        <p id="manual-rename-description" className="text-sm text-[var(--zc-text-secondary)]">{t(mode === "move" ? "libraryMoveDescription" : "libraryRenameDescription")}</p>
        <p className="break-all text-sm">{file.name}</p>
        {mode === "move" ? <div className="grid min-w-0 gap-2 text-sm">
          <span>{t("libraryMoveDestination")}</span>
          <p className="break-all">{directory || t("libraryMoveNoDestination")}</p>
          <button type="button" className={buttonSecondary} disabled={busy} onClick={() => void chooseDirectory()}>{t("libraryMoveChooseDestination")}</button>
        </div> : null}
        <label className="grid gap-2 text-sm">{t("libraryRenameNewName")}
          <input ref={inputRef} className={inputSurface} value={name} disabled={busy}
            onChange={(event) => { setName(event.target.value); setBlocked(null); setError(""); }}
            onKeyDown={(event) => { if (event.key === "Enter" && (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229)) event.preventDefault(); }}
            aria-invalid={Boolean(error)} aria-describedby={error ? "manual-rename-error" : undefined} />
        </label>
        {error ? <p id="manual-rename-error" role="alert" className="break-words text-sm text-[var(--zc-danger-text)]">{error}</p> : null}
        {blocked ? <div role="alert" className="grid min-w-0 gap-2 text-sm text-[var(--zc-danger-text)]">
          <p>{blocked.blocking_reason === "manual_file_target_exists" ? t("libraryManualConflict") : blocked.blocking_reason || t("operationBlocked")}</p>
          <p className="break-all">{blocked.target_path}</p>
          <p>{t(mode === "move" ? "libraryManualConflictNext" : "libraryRenameConflictNext")}</p>
        </div> : null}
        <div className="flex flex-wrap justify-end gap-2">
          <button type="button" className={buttonSecondary} onClick={close}>{t("cancel")}</button>
          <button type="submit" className={buttonPrimary} disabled={busy || !name.trim() || (mode === "rename" ? name.trim() === file.name : !directory)}>
            {busy ? t("loading") : t(mode === "move" ? "libraryMovePreview" : "libraryRenamePreview")}
          </button>
        </div>
      </form>
    </div>
  </ModalPortal>;
}
