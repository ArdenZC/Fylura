import { useEffect, useMemo, useRef, useState, type ChangeEvent } from "react";
import { ListOrdered, RefreshCw, SkipForward, Sparkles } from "lucide-react";
import { tauriApi } from "../../../api/tauriApi";
import { ModalPortal } from "../../../components/modal/ModalPortal";
import type { FileLibrarySummary, OperationPreview } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonPrimary, buttonSecondary, cn, inputSurface, raisedSurface } from "../../../utils/tw";
import { localizedStableError, readableError } from "../../../utils/viewHelpers";
import {
  BATCH_RENAME_LIMIT,
  buildBatchRenameName,
  keepBothCandidate,
  validateBatchRenameName,
  type BatchRenameMode,
  type BatchRenameTemplate
} from "./batchRenameModel";

type Props = {
  files: Pick<FileLibrarySummary, "id" | "name">[];
  t: Translator;
  onClose: () => void;
  onPreview: (previews: OperationPreview[]) => void;
  restoreFocus: () => HTMLElement | null;
};

const PREVIEW_CONCURRENCY = 6;

type BatchRenameErrorKey =
  | "libraryBatchRenameFindRequired"
  | "libraryBatchRenamePatternRequired"
  | "libraryBatchRenamePatternInvalid"
  | "libraryBatchRenameNumberInvalid"
  | "libraryBatchRenameNameEmpty"
  | "libraryBatchRenameNameUnsafe"
  | "libraryBatchRenameNameUnchanged"
  | "libraryBatchRenameDuplicate";

function validationKey(error: "empty" | "unsafe" | "unchanged"): BatchRenameErrorKey {
  if (error === "empty") return "libraryBatchRenameNameEmpty";
  if (error === "unsafe") return "libraryBatchRenameNameUnsafe";
  return "libraryBatchRenameNameUnchanged";
}

export function BatchRenameDialog({ files, t, onClose, onPreview, restoreFocus }: Props) {
  const triggerRef = useRef<HTMLInputElement | null>(null);
  const alive = useRef(true);
  const requestEpoch = useRef(0);
  const [mode, setMode] = useState<BatchRenameMode>("number");
  const [prefix, setPrefix] = useState("");
  const [startText, setStartText] = useState("1");
  const [widthText, setWidthText] = useState("2");
  const [find, setFind] = useState("");
  const [replacement, setReplacement] = useState("");
  const [pattern, setPattern] = useState("");
  const [overrides, setOverrides] = useState<Record<string, string>>({});
  const [skippedIds, setSkippedIds] = useState<Set<string>>(() => new Set());
  const [previews, setPreviews] = useState<Record<string, OperationPreview>>({});
  const [previewErrors, setPreviewErrors] = useState<Record<string, string>>({});
  const [previewProgress, setPreviewProgress] = useState<{ processed: number; total: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      requestEpoch.current++;
    };
  }, []);

  const startNumber = Number(startText);
  const widthNumber = Number(widthText);
  const regexInvalid = mode === "regex" && pattern.length > 0 && (() => {
    try { new RegExp(pattern, "g"); return false; }
    catch { return true; }
  })();
  const templateError: BatchRenameErrorKey | null = mode === "number" && (!Number.isInteger(startNumber) || startNumber < 0 || startNumber > 999_999_999 || !Number.isInteger(widthNumber) || widthNumber < 1 || widthNumber > 8)
    ? "libraryBatchRenameNumberInvalid"
    : mode === "replace" && !find
    ? "libraryBatchRenameFindRequired"
    : mode === "regex" && !pattern
      ? "libraryBatchRenamePatternRequired"
      : regexInvalid
        ? "libraryBatchRenamePatternInvalid"
      : null;
  const template = useMemo<BatchRenameTemplate>(() => ({
    mode,
    prefix,
    start: Number.isInteger(startNumber) ? startNumber : 0,
    width: Number.isInteger(widthNumber) ? widthNumber : 1,
    find,
    replacement,
    pattern
  }), [find, mode, pattern, prefix, replacement, startText, widthText]);

  const rows = useMemo(() => {
    const base = files.map((file, index) => {
      let generatedName = overrides[file.id];
      let generationError: BatchRenameErrorKey | null = null;
      if (generatedName === undefined) {
        if (templateError) generatedName = file.name;
        else {
          try { generatedName = buildBatchRenameName(file.name, index, template); }
          catch { generationError = "libraryBatchRenamePatternInvalid"; }
        }
      }
      const name = generatedName ?? "";
      const validation = generationError ? null : validateBatchRenameName(file.name, name);
      return {
        file,
        name,
        localError: templateError ? null : generationError ?? (validation ? validationKey(validation) : null)
      };
    });
    const duplicates = new Set<string>();
    const idsByTarget = new Map<string, string[]>();
    for (const row of base) {
      if (skippedIds.has(row.file.id) || row.localError || !row.name) continue;
      const key = row.name.normalize("NFC").toLocaleLowerCase("en-US");
      const existing = idsByTarget.get(key);
      if (existing) existing.push(row.file.id);
      else idsByTarget.set(key, [row.file.id]);
    }
    for (const ids of idsByTarget.values()) if (ids.length > 1) ids.forEach((id) => duplicates.add(id));
    return base.map((row) => ({
      ...row,
      localError: row.localError ?? (duplicates.has(row.file.id) ? "libraryBatchRenameDuplicate" as const : null)
    }));
  }, [files, overrides, skippedIds, template, templateError]);

  const readyRows = rows.filter((row) => !skippedIds.has(row.file.id));
  const executableCount = readyRows.filter((row) => {
    const preview = previews[row.file.id];
    return preview && preview.is_executable !== false && !preview.blocking_reason;
  }).length;
  const canContinue = readyRows.length > 0
    && executableCount > 0
    && readyRows.every((row) => !row.localError && !previewErrors[row.file.id] && previews[row.file.id]);

  function invalidatePreviews() {
    requestEpoch.current++;
    setPreviews({});
    setPreviewErrors({});
    setPreviewProgress(null);
    setError("");
  }

  function changeTemplate(update: () => void) {
    update();
    setOverrides({});
    setSkippedIds(new Set());
    invalidatePreviews();
  }

  function setRowName(fileId: string, value: string) {
    setOverrides((current) => ({ ...current, [fileId]: value }));
    setPreviews((current) => { const next = { ...current }; delete next[fileId]; return next; });
    setPreviewErrors((current) => { const next = { ...current }; delete next[fileId]; return next; });
    setError("");
  }

  function toggleSkipped(fileId: string, checked: boolean) {
    setSkippedIds((current) => {
      const next = new Set(current);
      if (checked) next.add(fileId); else next.delete(fileId);
      return next;
    });
    setPreviewErrors({});
    setPreviewProgress(null);
    setError("");
  }

  function describePreviewError(failure: unknown) {
    const message = readableError(failure);
    if (message.includes("manual_rename_name_unchanged")) return t("libraryRenameUnchanged");
    if (message.includes("manual_rename_source_read_only")) return t("libraryRenameReadOnly");
    if (message.includes("manual_rename_source_unavailable")) return t("libraryRenameUnavailable");
    return localizedStableError(failure, t);
  }

  async function generatePreviews() {
    if (busy || templateError || files.length > BATCH_RENAME_LIMIT || readyRows.length === 0) return;
    const epoch = ++requestEpoch.current;
    setBusy(true);
    setError("");
    setPreviewProgress({ processed: 0, total: readyRows.length });
    const nextPreviews: Record<string, OperationPreview> = {};
    const nextErrors: Record<string, string> = {};
    let processed = 0;
    const markProcessed = () => {
      processed++;
      if (alive.current && requestEpoch.current === epoch && (processed % 10 === 0 || processed === readyRows.length)) {
        setPreviewProgress({ processed, total: readyRows.length });
      }
    };
    let cursor = 0;
    const worker = async () => {
      while (cursor < readyRows.length && alive.current && requestEpoch.current === epoch) {
        const row = readyRows[cursor++];
        if (!row) continue;
        if (row.localError) {
          nextErrors[row.file.id] = t(row.localError);
          markProcessed();
          continue;
        }
        try {
          nextPreviews[row.file.id] = await tauriApi.getManualRenameOperationPreview(row.file.id, row.name);
        } catch (failure) {
          nextErrors[row.file.id] = describePreviewError(failure);
        }
        markProcessed();
      }
    };
    try {
      await Promise.all(Array.from({ length: Math.min(PREVIEW_CONCURRENCY, readyRows.length) }, () => worker()));
      if (alive.current && requestEpoch.current === epoch) {
        setPreviews(nextPreviews);
        setPreviewErrors(nextErrors);
      }
    } catch (failure) {
      if (alive.current && requestEpoch.current === epoch) setError(localizedStableError(failure, t));
    } finally {
      if (alive.current && requestEpoch.current === epoch) {
        setPreviewProgress(null);
        setBusy(false);
      }
    }
  }

  async function keepBoth(fileId: string, initialName: string) {
    if (busy) return;
    const epoch = ++requestEpoch.current;
    setBusy(true);
    setError("");
    setPreviewProgress(null);
    try {
      for (let ordinal = 2; ordinal <= 101; ordinal++) {
        if (!alive.current || requestEpoch.current !== epoch) return;
        const candidate = keepBothCandidate(initialName, ordinal);
        const preview = await tauriApi.getManualRenameOperationPreview(fileId, candidate);
        if (preview.is_executable !== false && !preview.blocking_reason) {
          if (alive.current && requestEpoch.current === epoch) {
            setOverrides((current) => ({ ...current, [fileId]: candidate }));
            setPreviews((current) => ({ ...current, [fileId]: preview }));
            setPreviewErrors((current) => { const next = { ...current }; delete next[fileId]; return next; });
          }
          return;
        }
      }
      if (alive.current && requestEpoch.current === epoch) setError(t("libraryBatchRenameKeepBothUnavailable"));
    } catch (failure) {
      if (alive.current && requestEpoch.current === epoch) setError(describePreviewError(failure));
    } finally {
      if (alive.current && requestEpoch.current === epoch) {
        setPreviewProgress(null);
        setBusy(false);
      }
    }
  }

  function continueToOperationPreview() {
    if (!canContinue || busy) return;
    const authoritative = readyRows.flatMap((row) => previews[row.file.id] ? [previews[row.file.id]!] : []);
    if (!authoritative.length) return;
    requestEpoch.current++;
    onPreview(authoritative);
  }

  function handleModeChange(event: ChangeEvent<HTMLSelectElement>) {
    changeTemplate(() => setMode(event.target.value as BatchRenameMode));
  }

  const statusFor = (fileId: string, localError: BatchRenameErrorKey | null) => {
    if (skippedIds.has(fileId)) return t("libraryBatchRenameSkipped");
    if (localError) return t(localError);
    if (previewErrors[fileId]) return previewErrors[fileId];
    const preview = previews[fileId];
    if (!preview) return t("libraryBatchRenameNotPreviewed");
    if (preview.is_executable === false || preview.blocking_reason) return t("libraryBatchRenameBlocked");
    return t("libraryBatchRenameReady");
  };

  return <ModalPortal initialFocusRef={triggerRef} restoreFocus={restoreFocus} onEscape={onClose}>
    <div className="fixed inset-0 z-50 grid place-items-center overflow-y-auto bg-[var(--zc-overlay)] p-3 sm:p-5">
      <section className={cn(raisedSurface, "grid max-h-[min(92vh,900px)] w-full max-w-5xl grid-rows-[auto_auto_1fr_auto] gap-4 overflow-hidden p-4 sm:p-5")} role="dialog" aria-modal="true" aria-labelledby="batch-rename-title" aria-busy={busy}>
        <header className="grid gap-1">
          <h2 id="batch-rename-title" className="flex items-center gap-2 text-lg font-semibold"><ListOrdered size={19}/>{t("libraryBatchRenameTitle")}</h2>
          <p className="text-sm text-[var(--zc-text-secondary)]">{t("libraryBatchRenameDescription").replace("{count}", String(files.length))}</p>
          {files.length > BATCH_RENAME_LIMIT ? <p role="alert" className="text-sm text-[var(--zc-danger-text)]">{t("libraryBatchRenameLimit").replace("{limit}", String(BATCH_RENAME_LIMIT))}</p> : null}
        </header>

        <div className="grid gap-3 rounded-[var(--zc-radius-control)] border border-[var(--zc-divider)] p-3 sm:grid-cols-[minmax(150px,0.7fr)_minmax(0,2fr)]">
          <label className="grid gap-1 text-sm">{t("libraryBatchRenameMode")}
            <select className={inputSurface} value={mode} disabled={busy} onChange={handleModeChange}>
              <option value="number">{t("libraryBatchRenameNumberMode")}</option>
              <option value="replace">{t("libraryBatchRenameReplaceMode")}</option>
              <option value="regex">{t("libraryBatchRenameRegexMode")}</option>
            </select>
          </label>
          {mode === "number" ? <div className="grid gap-2 sm:grid-cols-[2fr_1fr_1fr]">
            <label className="grid gap-1 text-sm">{t("libraryBatchRenamePrefix")}<input className={inputSurface} value={prefix} disabled={busy} maxLength={120} onChange={(event) => changeTemplate(() => setPrefix(event.target.value))}/></label>
            <label className="grid gap-1 text-sm">{t("libraryBatchRenameStart")}<input className={inputSurface} type="number" min="0" max="999999999" value={startText} disabled={busy} onChange={(event) => changeTemplate(() => setStartText(event.target.value))}/></label>
            <label className="grid gap-1 text-sm">{t("libraryBatchRenameWidth")}<input className={inputSurface} type="number" min="1" max="8" value={widthText} disabled={busy} onChange={(event) => changeTemplate(() => setWidthText(event.target.value))}/></label>
          </div> : mode === "replace" ? <div className="grid gap-2 sm:grid-cols-2">
            <label className="grid gap-1 text-sm">{t("libraryBatchRenameFind")}<input className={inputSurface} value={find} disabled={busy} maxLength={120} onChange={(event) => changeTemplate(() => setFind(event.target.value))}/></label>
            <label className="grid gap-1 text-sm">{t("libraryBatchRenameReplacement")}<input className={inputSurface} value={replacement} disabled={busy} maxLength={120} onChange={(event) => changeTemplate(() => setReplacement(event.target.value))}/></label>
          </div> : <div className="grid gap-2 sm:grid-cols-2">
            <label className="grid gap-1 text-sm">{t("libraryBatchRenamePattern")}<input className={inputSurface} value={pattern} disabled={busy} maxLength={120} onChange={(event) => changeTemplate(() => setPattern(event.target.value))}/></label>
            <label className="grid gap-1 text-sm">{t("libraryBatchRenameReplacement")}<input className={inputSurface} value={replacement} disabled={busy} maxLength={120} onChange={(event) => changeTemplate(() => setReplacement(event.target.value))}/></label>
          </div>}
          {templateError ? <p role="alert" className="text-sm text-[var(--zc-danger-text)] sm:col-span-2">{t(templateError)}</p> : null}
        </div>

        <div className="min-h-0 overflow-auto rounded-[var(--zc-radius-control)] border border-[var(--zc-divider)]" aria-label={t("libraryBatchRenameRows")}>
          <div className="grid min-w-[720px] grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(160px,0.8fr)_auto] gap-2 border-b border-[var(--zc-divider)] bg-[var(--zc-neutral-soft)] px-3 py-2 text-xs font-semibold text-[var(--zc-text-secondary)]">
            <span>{t("libraryBatchRenameSource")}</span><span>{t("libraryBatchRenameTarget")}</span><span>{t("libraryBatchRenameStatus")}</span><span>{t("libraryBatchRenameSkip")}</span>
          </div>
          <ul className="grid min-w-[720px] divide-y divide-[var(--zc-divider)]">
            {rows.map((row) => {
              const preview = previews[row.file.id];
              const rowError = previewErrors[row.file.id];
              const isBlocked = Boolean(preview && (preview.is_executable === false || preview.blocking_reason));
              return <li key={row.file.id} className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(160px,0.8fr)_auto] items-center gap-2 px-3 py-2" data-batch-rename-status={skippedIds.has(row.file.id) ? "skipped" : isBlocked ? "blocked" : preview ? "ready" : "pending"}>
                <span className="min-w-0 break-all text-sm" title={row.file.name}>{row.file.name}</span>
                <input className={inputSurface} value={row.name} disabled={busy} aria-label={`${t("libraryBatchRenameTarget")}: ${row.file.name}`} aria-invalid={Boolean(row.localError || rowError || isBlocked)} onChange={(event) => setRowName(row.file.id, event.target.value)}/>
                <div className="grid min-w-0 gap-1 text-xs"><span className={cn("break-words", row.localError || rowError || isBlocked ? "text-[var(--zc-danger-text)]" : "text-[var(--zc-text-secondary)]")}>{statusFor(row.file.id, row.localError)}</span>{isBlocked && preview?.target_path ? <span className="break-all" title={preview.target_path}>{preview.target_path}</span> : null}
                  {isBlocked && preview?.blocking_reason === "manual_file_target_exists" && !skippedIds.has(row.file.id) ? <button type="button" className="justify-self-start text-[var(--zc-primary)] underline" disabled={busy} onClick={() => void keepBoth(row.file.id, row.name)}><Sparkles size={12} className="mr-1 inline"/>{t("libraryBatchRenameKeepBoth")}</button> : null}
                </div>
                <label className="flex items-center gap-1 text-xs"><input type="checkbox" checked={skippedIds.has(row.file.id)} disabled={busy} aria-label={`${t("libraryBatchRenameSkip")}: ${row.file.name}`} onChange={(event) => toggleSkipped(row.file.id, event.target.checked)}/>{t("libraryBatchRenameSkip")}</label>
              </li>;
            })}
          </ul>
        </div>

        <footer className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--zc-divider)] pt-3">
          <div className="grid min-w-0 gap-1">
            {error ? <p role="alert" className="break-words text-sm text-[var(--zc-danger-text)]">{error}</p> : null}
            <p className="text-xs text-[var(--zc-text-secondary)]" role="status">{previewProgress ? `${t("libraryBatchRenameProgress")} ${previewProgress.processed}/${previewProgress.total} · ` : ""}{t("libraryBatchRenameSummary").replace("{ready}", String(executableCount)).replace("{total}", String(files.length)).replace("{skipped}", String(skippedIds.size))}</p>
          </div>
          <div className="flex flex-wrap justify-end gap-2">
            <button type="button" className={buttonSecondary} disabled={busy} onClick={onClose}>{t("cancel")}</button>
            <button type="button" className={buttonSecondary} disabled={busy || Boolean(templateError) || files.length > BATCH_RENAME_LIMIT || readyRows.length === 0} onClick={() => void generatePreviews()}><RefreshCw size={14}/>{busy ? t("loading") : t("libraryBatchRenameReview")}</button>
            <button type="button" className={buttonPrimary} disabled={busy || !canContinue} onClick={continueToOperationPreview}><SkipForward size={14}/>{t("libraryBatchRenameContinue")}</button>
          </div>
        </footer>
      </section>
    </div>
  </ModalPortal>;
}
