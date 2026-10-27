import { useEffect, useState } from "react";
import { libraryApi } from "../../../api/libraryApi";
import type { FilePersonalMetadata, FilePersonalMetadataResponse } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonPrimary, buttonSecondary, cn } from "../../../utils/tw";
import { SideSheet } from "../../shared/ui";

export function FilePersonalMetadataEditor({ fileId, t }: { fileId: string; t: Translator }) {
  const [metadata, setMetadata] = useState<FilePersonalMetadata | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  useEffect(() => {
    let active = true;
    setError(false);
    setMetadata(null);
    libraryApi.personalMetadata({ kind: "get", fileId }).then(({ metadata: result }) => {
      if (active) { setMetadata(result); setDraft(result?.note ?? ""); }
    }).catch(() => { if (active) setError(true); });
    return () => { active = false; };
  }, [fileId, reload]);
  async function save(note: string, favorite: boolean, replaceDraft: boolean) {
    if (!metadata || busy) return;
    setBusy(true);
    setError(false);
    try {
      const result = await libraryApi.personalMetadata({ kind: "update", fileId, note, favorite, expectedRevision: metadata.revision });
      setMetadata(result.metadata);
      if (replaceDraft) setDraft(result.metadata?.note ?? "");
    } catch { setError(true); }
    finally { setBusy(false); }
  }
  return <section className="grid gap-2 border-y border-[var(--zc-divider)] py-3" aria-label={t("u4Notes")} data-u4-personal>
    <p className="text-xs text-[var(--zc-text-tertiary)]">{t("u4Owner")}</p>
    {error ? <div role="alert" className="text-sm text-[var(--zc-danger-text)]">{t("u4PersonalError")}<button className={buttonSecondary} disabled={busy} onClick={() => setReload((value) => value + 1)}>{t("u4Reload")}</button></div> : null}
    {metadata ? <>
      <button className={buttonSecondary} disabled={!metadata.editable || busy || error} aria-pressed={metadata.favorite} onClick={() => void save(metadata.note, !metadata.favorite, false)}>{t(metadata.favorite ? "u4Unfavorite" : "u4Favorite")}</button>
      <label className="text-sm font-semibold">{t("u4Notes")}<textarea className="mt-2 block w-full resize-y rounded border border-[var(--zc-divider)] bg-[var(--zc-surface)] p-2 text-sm font-normal text-[var(--zc-text-primary)]" rows={4} maxLength={16000} value={draft} readOnly={!metadata.editable} disabled={busy} onChange={(event) => setDraft(event.target.value)} /></label>
      {!metadata.editable ? <p className="text-xs text-[var(--zc-text-secondary)]">{t("u4PersonalUnavailable")}</p> : <div className="flex flex-wrap gap-2">
        <button className={buttonPrimary} disabled={busy || error || draft === metadata.note} onClick={() => void save(draft, metadata.favorite, true)}>{t("u4Save")}</button>
        <button className={buttonSecondary} disabled={busy || error || !metadata.note} onClick={() => void save("", metadata.favorite, true)}>{t("u4Delete")}</button>
        {draft !== metadata.note ? <button className={buttonSecondary} disabled={busy} onClick={() => setDraft(metadata.note)}>{t("u4Cancel")}</button> : <span role="status" className="self-center text-xs">{t("u4Saved")}</span>}
      </div>}
    </> : !error ? <p role="status">{t("libraryLoadingResults")}</p> : null}
  </section>;
}

export function PersonalFiles({ open, onClose, onSelect, restoreFocus, t }: { open: boolean; onClose: () => void; onSelect: (id: string) => void; restoreFocus: () => HTMLElement | null; t: Translator }) {
  const [favorites, setFavorites] = useState(true);
  const [offset, setOffset] = useState(0);
  const [result, setResult] = useState<FilePersonalMetadataResponse | null>(null);
  const [error, setError] = useState(false);
  const [reload, setReload] = useState(0);
  useEffect(() => {
    if (!open) return;
    let active = true;
    setResult(null); setError(false);
    libraryApi.personalMetadata({ kind: "list", favorites, offset }).then((value) => { if (active) setResult(value); }).catch(() => { if (active) setError(true); });
    return () => { active = false; };
  }, [open, favorites, offset, reload]);
  return <SideSheet open={open} title={t("u4MyFiles")} description={t("u4Owner")} onClose={onClose} closeLabel={t("fileLibraryContextClose")} modalId="u4-personal-files" restoreFocus={restoreFocus}>
    <div className="grid gap-4 p-4">
      <div className="flex flex-wrap gap-2">{[true, false].map((value) => <button key={String(value)} className={cn(buttonSecondary)} aria-pressed={favorites === value} onClick={() => { setFavorites(value); setOffset(0); }}>{t(value ? "u4Favorites" : "u4WithNotes")}</button>)}</div>
      {error ? <div role="alert">{t("u4PersonalError")}<button className={buttonSecondary} onClick={() => setReload((value) => value + 1)}>{t("u4Reload")}</button></div> : result ? result.items.length ? <ul className="grid gap-2">{result.items.map((file) => <li key={file.fileId}><button className={cn(buttonSecondary, "w-full justify-start break-all text-left")} onClick={() => onSelect(file.fileId)}>{file.name}{file.isStale ? ` · ${t("libraryFileNotFound")}` : ""}</button></li>)}</ul> : <p>{t("u4Empty")}</p> : <p role="status">{t("libraryLoadingResults")}</p>}
      <div className="flex gap-2"><button className={buttonSecondary} disabled={!offset || !result} onClick={() => setOffset((value) => Math.max(0, value - 40))}>{t("u4Back")}</button><button className={buttonSecondary} disabled={!result?.hasMore} onClick={() => setOffset((value) => value + 40)}>{t("u4More")}</button></div>
    </div>
  </SideSheet>;
}
