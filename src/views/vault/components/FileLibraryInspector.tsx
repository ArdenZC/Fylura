import { convertFileSrc } from "@tauri-apps/api/core";
import { Info, TriangleAlert, X } from "lucide-react";
import { useEffect, useRef, useState, type MouseEvent } from "react";
import { FilePersonalMetadataEditor } from "../../fileLibrary/context/FilePersonalMetadata";
import { libraryApi } from "../../../api/libraryApi";
import { useRuntimeCapabilitiesContext } from "../../../contexts/AppContexts";
import type { FileLibraryDetail, FileLibrarySelectionSummary, FileLibrarySummary, LibrarySelectionV1, UserTag } from "../../../types/domain";
import type { Language } from "../../../i18n";
import type { Translator } from "../../../types/ui";
import { formatBytes, formatDate } from "../../../utils/format";
import { compactPath, formatDisplayPath } from "../../../utils/viewHelpers";
import { buttonSecondary, cn, overlaySurface, focusVisibleState, buttonPrimary, selectedFocusVisibleState, selectedSurface } from "../../../utils/tw";
import { ModalPortal } from "../../../components/modal/ModalPortal";
import { FileTypeIcon } from "../../../components/FileTypeIcon";
import { contentPolicyLabel, contentStatusLabel } from "./ContentUnderstandingSheet";
import { libraryRevealLabel } from "../../fileLibrary/presentation/fileLibraryPresentation";

export interface FileLibraryInspectorProps {
  selectedIds: ReadonlySet<string>;
  selectedFiles: FileLibrarySummary[];
  selectionKind: LibrarySelectionV1["kind"] | null;
  selectedCount: number | null;
  detail: FileLibraryDetail | null;
  selectionSummary: FileLibrarySelectionSummary | null;
  isLoading: boolean;
  error: string | null;
  language: Language;
  t: Translator;
  onPreview: (event: MouseEvent<HTMLButtonElement>, file: FileLibraryDetail) => void;
  onPreviewSelection?: (fileId: string, trigger: HTMLElement) => void;
  onReveal: (fileId: string) => void;
  onViewSuggestions: () => void;
  onViewOperations: () => void;
  onPermanentDelete?: (file: FileLibraryDetail) => void;
  onOpenContentUnderstanding: (file: FileLibraryDetail, trigger: HTMLElement) => void;
  onClearSelection: () => void;
  onRetryDetail: () => void;
  availableTags?: UserTag[];
  onToggleTag?: (tagId: string, operation: "add" | "remove") => void;
}

export function FileLibraryInspector({
  selectedIds,
  selectedFiles,
  selectionKind,
  selectedCount,
  detail,
  selectionSummary,
  isLoading,
  error,
  language,
  t,
  onPreview,
  onReveal,
  onViewSuggestions,
  onViewOperations,
  onPermanentDelete,
  onOpenContentUnderstanding,
  onClearSelection,
  onRetryDetail,
  availableTags = [],
  onToggleTag
}: FileLibraryInspectorProps) {
  void selectedIds;
  void selectedFiles;
  const contentKind = libraryInspectorContentKind(selectionKind, selectedCount);
  return (
    <aside className="min-h-0 overflow-auto border-l border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] p-4" aria-labelledby="library-inspector-title">
      <h2 id="library-inspector-title" className="text-base font-semibold text-[var(--zc-text-primary)]">{t("libraryInspector")}</h2>
      <div className="mt-3">
        {contentKind === "none" ? <EmptyInspector t={t} /> : null}
        {contentKind === "selection-summary" ? (
          <MultiInspector summary={selectionSummary} selectedCount={selectedCount} t={t} onViewSuggestions={onViewSuggestions} onViewOperations={onViewOperations} onClearSelection={onClearSelection} />
        ) : null}
        {contentKind === "inspector" ? (
          isLoading ? <LoadingInspector t={t} /> : error ? <DetailErrorInspector error={error} t={t} onRetry={onRetryDetail} /> : detail ? (
            <SingleInspector detail={detail} language={language} t={t} onPreview={onPreview} onReveal={onReveal} onViewSuggestions={onViewSuggestions} onViewOperations={onViewOperations} onPermanentDelete={onPermanentDelete} onOpenContentUnderstanding={onOpenContentUnderstanding} availableTags={availableTags} onToggleTag={onToggleTag} />
          ) : <MissingInspector t={t} />
        ) : null}
      </div>
    </aside>
  );
}

export function libraryInspectorContentKind(
  selectionKind: LibrarySelectionV1["kind"] | null,
  selectedCount: number | null
) {
  if (selectionKind === "all_matching") return "selection-summary" as const;
  if (selectionKind === "explicit" && selectedCount === 1) return "inspector" as const;
  if (selectionKind === "explicit" && selectedCount !== null && selectedCount > 1) return "selection-summary" as const;
  return "none" as const;
}

export function FileLibraryPreviewDialog({
  file,
  language,
  t,
  onClose,
  onReveal,
  restoreFocus
}: {
  file: FileLibraryDetail | null;
  language: Language;
  t: Translator;
  onClose: () => void;
  onReveal: (fileId: string) => void;
  restoreFocus?: () => HTMLElement | null;
}) {
  const closeRef = useRef<HTMLButtonElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  if (!file) return null;
  return (
    <ModalPortal initialFocusRef={closeRef} restoreFocus={restoreFocus} onEscape={() => onCloseRef.current()}>
      <div className="fixed inset-0 z-40 grid place-items-center bg-[var(--zc-overlay)] p-5" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onCloseRef.current(); }}>
        <section className={cn(overlaySurface, "grid w-full max-w-xl gap-4 p-5")} role="dialog" aria-modal="true" aria-labelledby="library-preview-title">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-xs font-semibold text-[var(--zc-text-tertiary)]">{previewTitle(file, t)}</p>
              <h2 id="library-preview-title" className="mt-1 truncate text-lg font-semibold text-[var(--zc-text-primary)]" title={file.name}>{file.name}</h2>
            </div>
            <button ref={closeRef} type="button" className="grid h-9 w-9 shrink-0 place-items-center rounded-[var(--zc-radius-control)] text-[var(--zc-text-secondary)] hover:bg-[var(--zc-surface-hover)]" aria-label={t("libraryPreviewClose")} title={t("libraryPreviewClose")} onClick={onCloseRef.current}>
              <X size={17} />
            </button>
          </div>
          <PreviewSurface file={file} t={t} />
          <div className="flex flex-wrap justify-end gap-2">
            <button className={buttonSecondary} onClick={() => onReveal(file.id)}>{libraryRevealLabel(t)}</button>
            <button className={buttonPrimary} onClick={onClose}>{t("libraryPreviewClose")}</button>
          </div>
          <p className="text-xs text-[var(--zc-text-tertiary)]">{formatDate(String(file.modifiedAt), language)} · {formatBytes(file.size)}</p>
        </section>
      </div>
    </ModalPortal>
  );
}

function EmptyInspector({ t }: { t: Translator }) {
  return <div className="grid min-h-40 place-items-center gap-3 border-y border-[var(--zc-divider)] py-6 text-center"><Info size={22} className="text-[var(--zc-info-text)]" aria-hidden="true" /><p className="max-w-xs text-sm leading-6 text-[var(--zc-text-secondary)]">{t("libraryInspectorEmpty")}</p></div>;
}

function LoadingInspector({ t }: { t: Translator }) {
  return <div className="grid min-h-40 place-items-center gap-3 border-y border-[var(--zc-divider)] py-6 text-center"><Info size={22} className="text-[var(--zc-info-text)]" aria-hidden="true" /><p className="max-w-xs text-sm leading-6 text-[var(--zc-text-secondary)]">{t("libraryLoadingResults")}</p></div>;
}

function MissingInspector({ t }: { t: Translator }) {
  return <div className="grid min-h-40 place-items-center gap-3 border-y border-[var(--zc-divider)] py-6 text-center"><Info size={22} className="text-[var(--zc-warning-text)]" aria-hidden="true" /><p className="max-w-xs text-sm leading-6 text-[var(--zc-text-secondary)]">{t("libraryFileNotFound")}</p></div>;
}

function DetailErrorInspector({ error, t, onRetry }: { error: string; t: Translator; onRetry: () => void }) {
  void error;
  return <div className="grid min-h-40 place-items-center gap-3 border-y border-[var(--zc-divider)] py-6 text-center"><TriangleAlert size={22} className="text-[var(--zc-danger-text)]" aria-hidden="true" /><div className="grid gap-1"><p className="text-sm font-semibold text-[var(--zc-text-primary)]">{t("libraryDetailLoadFailedTitle")}</p><p className="max-w-xs text-sm leading-6 text-[var(--zc-text-secondary)]">{t("libraryDetailLoadFailedDesc")}</p></div><button type="button" className={buttonSecondary} onClick={onRetry}>{t("libraryDetailRetry")}</button></div>;
}

function MultiInspector({ summary, selectedCount, t, onViewSuggestions, onViewOperations, onClearSelection }: { summary: FileLibrarySelectionSummary | null; selectedCount: number | null; t: Translator; onViewSuggestions: () => void; onViewOperations: () => void; onClearSelection: () => void }) {
  const count = summary?.count ?? selectedCount;
  return (
    <div className="grid gap-4">
      <div className="border-b border-[var(--zc-divider)] pb-3">
        <p className="text-lg font-semibold text-[var(--zc-text-primary)]">{count === null ? t("libraryCountPending") : t("librarySelectedCount").replace("{count}", String(count))}</p>
        <p className="mt-1 text-sm text-[var(--zc-text-secondary)]">{summary == null ? t("libraryCountPending") : t("librarySelectedTotalSize").replace("{size}", formatBytes(summary.totalSize))}</p>
      </div>
      <dl className="grid gap-3 text-sm">
        {summary?.typeCounts.length ? <div><dt className="text-xs font-semibold text-[var(--zc-text-tertiary)]">{t("librarySelectedTypes")}</dt><dd className="mt-1 text-[var(--zc-text-primary)]">{summary.typeCounts.map((item) => `${item.fileType} ×${item.count}`).join(" · ")}</dd></div> : null}
        {summary?.excludedCount ? <div><dt className="text-xs font-semibold text-[var(--zc-text-tertiary)]">{t("librarySelectedLoadedCount")}</dt><dd className="mt-1 text-[var(--zc-warning-text)]">{summary.excludedCount}</dd></div> : null}
      </dl>
      <div className="flex flex-wrap gap-2">
        <button className={buttonPrimary} onClick={onViewOperations}>{t("libraryReviewOperations")}</button>
        <button className={buttonSecondary} onClick={onViewSuggestions}>{t("libraryViewSuggestions")}</button>
        <button className="text-sm font-medium text-[var(--zc-text-secondary)] underline-offset-2 hover:underline" onClick={onClearSelection}>{t("libraryClearSelection")}</button>
      </div>
      <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]">{t("librarySelectionSafety")}</p>
    </div>
  );
}

function SingleInspector({ detail, language, t, onPreview, onReveal, onViewSuggestions, onViewOperations, onPermanentDelete, onOpenContentUnderstanding, availableTags, onToggleTag }: { detail: FileLibraryDetail; language: Language; t: Translator; onPreview: (event: MouseEvent<HTMLButtonElement>, file: FileLibraryDetail) => void; onReveal: (fileId: string) => void; onViewSuggestions: () => void; onViewOperations: () => void; onPermanentDelete?: (file: FileLibraryDetail) => void; onOpenContentUnderstanding: (file: FileLibraryDetail, trigger: HTMLElement) => void; availableTags: UserTag[]; onToggleTag?: (tagId: string, operation: "add" | "remove") => void }) {
  const missing = detail.isStale;
  const [showThumbnail, setShowThumbnail] = useState(false);
  const selectedTagIds = new Set(detail.tags.map((tag) => tag.id));
  return (
    <div className="grid gap-4">
      <div className="min-w-0 border-b border-[var(--zc-divider)] pb-3"><h3 className="break-words text-lg font-semibold text-[var(--zc-text-primary)]">{detail.name}</h3><p className="mt-1 text-sm text-[var(--zc-text-secondary)]">{detail.fileType} · {detail.purpose}</p></div>
      <dl className="grid grid-cols-2 gap-3 text-sm">
        <InspectorField label={t("libraryCurrentStatus")} value={missing ? t("libraryFileNotFound") : t("libraryReady")} tone={missing ? "warning" : "normal"} />
        <InspectorField label={t("fileLibraryContextSize")} value={formatBytes(detail.size)} />
        <InspectorField label={t("fileLibraryContextCreated")} value={formatDate(String(detail.createdAt), language)} />
        <InspectorField label={t("fileModified")} value={formatDate(String(detail.modifiedAt), language)} />
        <InspectorField label={t("fileLocation")} value={compactPath(formatDisplayPath(detail.path), 44)} title={formatDisplayPath(detail.path)} />
        {detail.nativeSemantics ? <>
          {detail.nativeSemantics.isPackage ? <InspectorField label={t("libraryNativePackage")} value={t("libraryNativePackageValue")} /> : null}
          <InspectorField label={t("libraryNativeBacking")} value={nativeBackingLabel(detail.nativeSemantics.cloudBacking, t)} />
          <InspectorField label={t("libraryNativeAvailability")} value={nativeAvailabilityLabel(detail.nativeSemantics.contentAvailability, t)} />
        </> : null}
      </dl>
      <div className="flex flex-wrap gap-2">{!missing ? <button type="button" className={buttonSecondary} onClick={(event) => onPreview(event, detail)}>{t("libraryPreview")}</button> : null}<button className={buttonSecondary} onClick={() => onReveal(detail.id)}>{libraryRevealLabel(t)}</button></div>
      <FilePersonalMetadataEditor key={detail.id} fileId={detail.id} t={t} />
      <details onToggle={(event) => setShowThumbnail(event.currentTarget.open)}><summary className="cursor-pointer text-sm font-semibold">{t("libraryPreview")}</summary>{showThumbnail ? <InspectorQuickLookPreview file={detail} t={t} /> : null}</details>
      <details><summary className="cursor-pointer text-sm font-semibold">{t("libraryClassification")}</summary><dl className="mt-3 grid gap-3 text-sm">
        <InspectorField label={t("libraryClassification")} value={detail.suggestedAction || t("unknown")} />
        <InspectorField label={t("lifecycle")} value={detail.lifecycle} />
        <InspectorField label={t("risk")} value={detail.risk} />
        <InspectorField label={t("libraryClassificationReason")} value={detail.classificationReason || t("unknown")} />
        <InspectorField label={t("confidence")} value={confidenceLabel(detail.confidence, t)} />
      </dl></details>
      <details><summary className="cursor-pointer text-sm font-semibold">{t("contentUnderstandingTitle")}</summary><section className="grid gap-2 border-t border-[var(--zc-divider)] pt-3" aria-labelledby="content-understanding-title">
        <div><h3 id="content-understanding-title" className="text-sm font-semibold text-[var(--zc-text-primary)]">{t("contentUnderstandingTitle")}</h3><p className="mt-1 text-xs leading-5 text-[var(--zc-text-secondary)]">{t("contentSourceUnchanged")}</p></div>
        <InspectorField label={t("contentStatus")} value={contentStatusLabel(detail.contentStatus, t)} />
        <InspectorField label={t("contentPolicy")} value={detail.contentPolicy ? contentPolicyLabel(detail.contentPolicy, t) : t("contentPolicyPerRoot")} />
        <button type="button" className={buttonSecondary} onClick={(event) => onOpenContentUnderstanding(detail, event.currentTarget)}>{t("contentOpen")}</button>
      </section></details>
      {availableTags.length ? <section className="grid gap-2 border-t border-[var(--zc-divider)] pt-3"><h3 className="text-xs font-semibold text-[var(--zc-text-tertiary)]">{t("libraryTags")}</h3><div className="flex flex-wrap gap-1.5">{availableTags.map((tag) => { const active = selectedTagIds.has(tag.id); return <button key={tag.id} type="button" className={cn("rounded-[var(--zc-radius-control)] border px-2 py-1 text-xs", active ? selectedFocusVisibleState : focusVisibleState, active ? selectedSurface : "border-[var(--zc-divider)] text-[var(--zc-text-secondary)]")} onClick={() => onToggleTag?.(tag.id, active ? "remove" : "add")} aria-pressed={active}>{tag.displayName}</button>; })}</div></section> : null}
      <div className="flex flex-wrap gap-2"><button className={buttonPrimary} onClick={onViewOperations}>{t("libraryReviewOperations")}</button>{onPermanentDelete && !missing ? <button className="text-sm font-medium text-[var(--zc-danger-text)] underline-offset-2 hover:underline" onClick={() => onPermanentDelete(detail)}>{t("libraryPermanentDelete")}</button> : null}<button className={buttonSecondary} onClick={onViewSuggestions}>{t("libraryViewSuggestions")}</button></div>
    </div>
  );
}

function InspectorQuickLookPreview({ file, t }: { file: FileLibraryDetail; t: Translator }) {
  const { capabilities } = useRuntimeCapabilitiesContext();
  const [state, setState] = useState<"idle" | "loading" | "ready" | "unavailable">("idle");
  const [source, setSource] = useState<string | null>(null);
  const requestEpoch = useRef(0);
  const quickLookAvailable = capabilities?.macosQuickLookThumbnailAvailable === true;

  useEffect(() => {
    const epoch = ++requestEpoch.current;
    setSource(null);
    if (file.isStale || !quickLookAvailable) {
      setState("idle");
      return () => undefined;
    }

    let disposed = false;
    const requestId = createQuickLookRequestId();
    const requestIdRefValue = requestId;
    setState("loading");
    void libraryApi.requestMacosThumbnail(file.id, 512, requestId)
      .then((thumbnailPath) => {
        if (disposed || requestEpoch.current !== epoch) return;
        if (!thumbnailPath.trim()) {
          setState("unavailable");
          return;
        }
        setSource(convertFileSrc(thumbnailPath));
        setState("ready");
      })
      .catch(() => {
        if (!disposed && requestEpoch.current === epoch) setState("unavailable");
      });

    return () => {
      disposed = true;
      void libraryApi.cancelMacosThumbnail(requestIdRefValue).catch(() => undefined);
    };
  }, [file.id, file.isStale, quickLookAvailable]);

  if (state === "ready" && source) {
    return (
      <div className="grid min-h-36 place-items-center gap-2 border-y border-[var(--zc-divider)] bg-[var(--zc-surface)] px-4 py-5 text-center" data-library-preview-kind="quick-look" aria-label={t("libraryQuickLookThumbnail")}>
        <img src={source} alt={t("libraryQuickLookThumbnail")} className="max-h-48 max-w-full rounded-[var(--zc-radius-control)] object-contain" />
        <span className="text-xs text-[var(--zc-text-secondary)]">{t("libraryQuickLookThumbnail")}</span>
      </div>
    );
  }
  if (state === "loading") {
    return <div className="grid min-h-36 place-items-center gap-2 border-y border-[var(--zc-divider)] bg-[var(--zc-surface)] px-4 py-5 text-center" data-library-preview-kind="quick-look" aria-busy="true" role="status"><Info size={24} className="text-[var(--zc-info-text)]" aria-hidden="true" /><span className="text-xs text-[var(--zc-text-secondary)]">{t("libraryQuickLookLoading")}</span></div>;
  }
  if (state === "unavailable") {
    return <div className="grid min-h-36 place-items-center gap-2 border-y border-[var(--zc-divider)] bg-[var(--zc-surface)] px-4 py-5 text-center" data-library-preview-kind="quick-look"><TriangleAlert size={24} className="text-[var(--zc-warning-text)]" aria-hidden="true" /><span className="max-w-xs text-xs leading-5 text-[var(--zc-text-secondary)]">{t("libraryQuickLookUnavailable")}</span></div>;
  }
  return <PreviewSurface file={file} t={t} />;
}

function createQuickLookRequestId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") return crypto.randomUUID();
  return `quick-look-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function InspectorField({ label, value, title, tone = "normal" }: { label: string; value: string; title?: string; tone?: "normal" | "warning" }) {
  return <div className="min-w-0"><dt className="text-xs font-semibold text-[var(--zc-text-tertiary)]">{label}</dt><dd className={cn("mt-0.5 truncate text-sm", tone === "warning" ? "text-[var(--zc-warning-text)]" : "text-[var(--zc-text-primary)]")} title={title ?? value}>{value}</dd></div>;
}

function PreviewSurface({ file, t }: { file: FileLibraryDetail; t: Translator }) {
  const missing = file.isStale;
  return <div className="grid min-h-36 place-items-center gap-2 border-y border-[var(--zc-divider)] bg-[var(--zc-surface)] px-4 py-5 text-center" data-library-preview-kind={file.extension.toLowerCase() === "pdf" ? "pdf" : "metadata"}>{missing ? <TriangleAlert size={30} className="text-[var(--zc-warning-text)]" aria-hidden="true" /> : <FileTypeIcon file={{ file_type: file.fileType as never, extension: file.extension }} size={30} className="text-[var(--zc-info-text)]" />}<strong className="text-sm text-[var(--zc-text-primary)]">{missing ? t("libraryFileUnavailableTitle") : previewTitle(file, t)}</strong><span className="max-w-xs text-xs leading-5 text-[var(--zc-text-secondary)]">{missing ? t("libraryFileUnavailableDesc") : t("libraryPreviewUnavailable")}</span></div>;
}

function previewTitle(file: FileLibraryDetail, t: Translator) {
  if (file.extension.toLowerCase() === "pdf") return t("libraryPreviewPdfFile");
  return t("libraryPreviewUnavailable");
}

function confidenceLabel(confidence: number, t: Translator) {
  if (confidence >= 0.8) return t("libraryConfidenceHigh");
  if (confidence >= 0.65) return t("libraryConfidenceMedium");
  return t("libraryConfidenceLow");
}

function nativeBackingLabel(backing: string, t: Translator) {
  switch (backing) {
    case "icloud":
      return t("libraryNativeICloud");
    case "file_provider":
      return t("libraryNativeFileProvider");
    case "local":
      return t("libraryNativeLocal");
    default:
      return t("libraryNativeUnknown");
  }
}

function nativeAvailabilityLabel(availability: string, t: Translator) {
  switch (availability) {
    case "local":
      return t("libraryNativeContentLocal");
    case "not_local":
      return t("libraryNativeContentNotLocal");
    case "downloading":
      return t("libraryNativeContentDownloading");
    case "metadata_only":
      return t("libraryNativeContentMetadataOnly");
    default:
      return t("libraryNativeContentUnknown");
  }
}
