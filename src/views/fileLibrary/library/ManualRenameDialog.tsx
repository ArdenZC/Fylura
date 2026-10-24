import { useEffect, useRef, useState, type FormEvent } from "react";
import { tauriApi } from "../../../api/tauriApi";
import { ModalPortal } from "../../../components/modal/ModalPortal";
import type { FileLibrarySummary, OperationPreview } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonPrimary, buttonSecondary, cn, inputSurface, raisedSurface } from "../../../utils/tw";
import { localizedStableError, readableError } from "../../../utils/viewHelpers";

export function ManualRenameDialog({ file, t, onClose, onPreview, restoreFocus }: {
  file: FileLibrarySummary;
  t: Translator;
  onClose: () => void;
  onPreview: (preview: OperationPreview) => void;
  restoreFocus: () => HTMLElement | null;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const active = useRef(true);
  const [name, setName] = useState(file.name);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    active.current = true;
    const dot = file.name.lastIndexOf(".");
    inputRef.current?.setSelectionRange(0, dot > 0 ? dot : file.name.length);
    return () => { active.current = false; };
  }, [file.name]);

  function close() {
    active.current = false;
    onClose();
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || !name.trim() || name.trim() === file.name) return;
    setBusy(true);
    setError("");
    try {
      const preview = await tauriApi.getManualRenameOperationPreview(file.id, name);
      if (active.current) onPreview(preview);
    } catch (failure) {
      if (!active.current) return;
      const message = readableError(failure);
      setError(message.includes("manual_rename_source_read_only") ? t("libraryRenameReadOnly")
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
        className={cn(raisedSurface, "grid w-full max-w-lg gap-4 p-5")}>
        <h2 id="manual-rename-title" className="text-lg font-semibold">{t("libraryManualRename")}</h2>
        <p id="manual-rename-description" className="text-sm text-[var(--zc-text-secondary)]">{t("libraryRenameDescription")}</p>
        <p className="break-all text-sm">{file.name}</p>
        <label className="grid gap-2 text-sm">{t("libraryRenameNewName")}
          <input ref={inputRef} className={inputSurface} value={name} disabled={busy}
            onChange={(event) => setName(event.target.value)}
            onKeyDown={(event) => { if (event.key === "Enter" && (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229)) event.preventDefault(); }}
            aria-invalid={Boolean(error)} aria-describedby={error ? "manual-rename-error" : undefined} />
        </label>
        {error ? <p id="manual-rename-error" role="alert" className="break-words text-sm text-[var(--zc-danger-text)]">{error}</p> : null}
        <div className="flex flex-wrap justify-end gap-2">
          <button type="button" className={buttonSecondary} onClick={close}>{t("cancel")}</button>
          <button type="submit" className={buttonPrimary} disabled={busy || !name.trim() || name.trim() === file.name}>
            {busy ? t("loading") : t("libraryRenamePreview")}
          </button>
        </div>
      </form>
    </div>
  </ModalPortal>;
}
