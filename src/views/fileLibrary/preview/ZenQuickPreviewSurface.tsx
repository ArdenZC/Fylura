import { Archive, File, FileText, Film, Folder, Image as ImageIcon, Info, Music, Pin, Table, X } from "lucide-react";
import { useCallback, useId, type RefObject } from "react";
import { tauriApi } from "../../../api/tauriApi";
import { useI18nContext } from "../../../contexts/AppContexts";
import { useAppStore } from "../../../store/useAppStore";
import { buttonSecondary, cn, overlaySurface } from "../../../utils/tw";
import { usePreviewExperience } from "./PreviewExperienceProvider";
import {
  metadataFromSnapshot,
  previewStateAnnouncement,
  renderPreviewBody,
  usePreviewImagePresentation,
  usePreviewPdfPresentation
} from "./PreviewContent";
import { PreviewNavigation } from "./PreviewNavigation";
import { previewPresentationState } from "./previewExperienceController";
import type { PreviewAssetRequest, PreviewNativePresentation, PreviewSnapshot } from "../../../types/fileWorkspace";
import { formatBytes, formatDate } from "../../../utils/format";

export function ZenQuickPreviewSurface({
  mode,
  surfaceRef,
  onClose
}: {
  mode: "floating" | "pinned";
  surfaceRef?: RefObject<HTMLElement | null>;
  onClose?: () => void;
}) {
  const { controller, state } = usePreviewExperience();
  const { language, t } = useI18nContext();
  // The floating host is portaled outside the app root, so the root's
  // data-density attribute is not an ancestor here; the card carries its own.
  const density = useAppStore((store) => store.density);
  const titleId = useId();
  const descriptionId = useId();
  const detailsOpen = state.detailsOpen;
  const requestPreviewAsset = useCallback(
    (request: PreviewAssetRequest) => controller.requestPreviewAsset(request),
    [controller]
  );
  const updateNativePreviewGeometry = useCallback(
    (previewId: string, presentation: PreviewNativePresentation) => controller.updateNativePreviewGeometry(previewId, presentation),
    [controller]
  );
  const imagePresentation = usePreviewImagePresentation(state.snapshot, state.source);
  const pdfPresentation = usePreviewPdfPresentation(state.snapshot, state.source);

  if (!state.visible || state.host !== mode) return null;

  const source = state.source;
  const metadata = metadataFromSnapshot(state.snapshot);
  const title = source?.displayName ?? (mode === "pinned" ? t("previewPinnedTitle") : t("previewHostTitle"));
  const description = source?.source === "browse" ? t("previewBrowseSource") : t("previewLibrarySource");
  const fileType = metadata?.mediaType ?? source?.typeHint ?? source?.extension;
  const fileSize = metadata?.sizeBytes ?? source?.size;
  const modifiedAt = metadata?.modifiedAtEpochMs ?? source?.modifiedAt;
  const materialization = metadata?.materialization ?? source?.materialization;
  const canReveal = source?.previewSource.kind === "managed" && Boolean(state.snapshot?.effectiveCapabilities.canReveal);
  const navigationLabel = state.navigation === null
    ? fileType ?? title
    : (fileType ?? title) + " · " + (state.navigation.currentIndex + 1) + " / " + state.navigation.loadedCount;
  const closePreview = onClose ?? (() => controller.close("button"));

  async function revealCurrentFile() {
    if (!canReveal || source?.previewSource.kind !== "managed") return;
    await tauriApi.revealFileLibraryEntry(source.previewSource.fileId).catch(() => undefined);
  }

  return (
    <section
      className={cn(overlaySurface, "zc-quick-preview-card")}
      role="dialog"
      aria-modal={mode === "floating" ? "true" : "false"}
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      data-preview-card="true"
      data-density={density}
      data-preview-surface-mode={mode}
      data-preview-details-open={detailsOpen ? "true" : "false"}
      data-preview-state={state.phase}
      data-preview-content-state={previewPresentationState(state.phase, state.snapshot, imagePresentation.state, pdfPresentation.state)}
      data-preview-epoch={state.frontendEpoch}
      data-preview-source={source?.source ?? "none"}
      data-preview-identity={source?.previewSource.kind === "managed"
        ? source.previewSource.fileId
        : source?.previewSource.kind === "ephemeral"
          ? source.previewSource.browseSessionId + ":" + source.previewSource.entryId
          : "none"}
      ref={surfaceRef}
      tabIndex={-1}
    >
      <QuickPreviewHeader
        mode={mode}
        title={title}
        typeIcon={previewTypeIcon(source?.extension, fileType, source?.entryKind)}
        navigationLabel={navigationLabel}
        titleId={titleId}
        descriptionId={descriptionId}
        detailsOpen={detailsOpen}
        onDetailsToggle={() => controller.setDetailsOpen(!detailsOpen)}
        onUnpin={() => void controller.unpin()}
        onPin={() => void controller.pin()}
        onClose={closePreview}
        t={t}
      />
      <div
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="true"
        data-preview-state-announcement="true"
      >
        {previewStateAnnouncement(state.phase, t, state.snapshot, imagePresentation.state, pdfPresentation.state)}
      </div>
      <QuickPreviewViewport
        detailsOpen={detailsOpen}
        source={source}
        metadata={metadata}
        language={language}
        t={t}
        state={state}
        requestPreviewAsset={requestPreviewAsset}
        updateNativePreviewGeometry={updateNativePreviewGeometry}
        imagePresentation={imagePresentation}
        pdfPresentation={pdfPresentation}
        fileType={fileType}
        fileSize={fileSize}
        modifiedAt={modifiedAt}
        materialization={materialization}
        canReveal={canReveal}
        onReveal={() => void revealCurrentFile()}
      />
    </section>
  );
}

function QuickPreviewHeader({
  mode,
  title,
  typeIcon: TypeIcon,
  navigationLabel,
  titleId,
  descriptionId,
  detailsOpen,
  onDetailsToggle,
  onUnpin,
  onPin,
  onClose,
  t
}: {
  mode: "floating" | "pinned";
  title: string;
  /** File-type glyph for the left-aligned identity block. */
  typeIcon: typeof File;
  navigationLabel: string;
  titleId: string;
  descriptionId: string;
  detailsOpen: boolean;
  onDetailsToggle: () => void;
  onUnpin: () => void;
  onPin: () => void;
  onClose: () => void;
  t: ReturnType<typeof useI18nContext>["t"];
}) {
  return (
    <header className="zc-quick-preview-header">
      <div className="zc-quick-preview-identity">
        <span className="zc-quick-preview-identity-icon" aria-hidden="true">
          <TypeIcon size={16} strokeWidth={1.7} />
        </span>
        <div className="zc-quick-preview-header-title min-w-0">
          <h2 id={titleId} className="zc-quick-preview-title" title={title}>{title}</h2>
          <p className="zc-quick-preview-title-meta" title={navigationLabel}>{navigationLabel}</p>
          <p id={descriptionId} className="sr-only">{t("previewFileInfo")}</p>
        </div>
      </div>
      <div className="zc-quick-preview-header-navigation">
        <PreviewNavigation compact />
      </div>
      <div className="zc-quick-preview-header-actions">
        <button
          type="button"
          className="zc-quick-preview-action"
          aria-label={t("previewFileInfo")}
          title={t("previewFileInfo")}
          aria-expanded={detailsOpen}
          aria-controls="quick-preview-details"
          data-preview-details-toggle="true"
          onClick={onDetailsToggle}
        >
          <Info size={15} aria-hidden="true" />
          <span className="sr-only">{t("previewFileInfo")}</span>
        </button>
        <button
          type="button"
          className="zc-quick-preview-action"
          aria-label={mode === "pinned" ? t("previewUnpin") : t("previewPin")}
          title={mode === "pinned" ? t("previewUnpin") : t("previewPin")}
          aria-pressed={mode === "pinned"}
          data-preview-pin="true"
          data-preview-pin-state={mode}
          onClick={mode === "pinned" ? onUnpin : onPin}
        >
          <Pin size={15} aria-hidden="true" />
          <span className="sr-only">{mode === "pinned" ? t("previewUnpin") : t("previewPin")}</span>
        </button>
        <button
          type="button"
          className="zc-quick-preview-close"
          aria-label={t("libraryPreviewClose")}
          title={t("libraryPreviewClose")}
          onClick={onClose}
        >
          <X size={17} aria-hidden="true" />
        </button>
      </div>
    </header>
  );
}

function QuickPreviewViewport({
  detailsOpen,
  source,
  metadata,
  language,
  t,
  state,
  requestPreviewAsset,
  updateNativePreviewGeometry,
  imagePresentation,
  pdfPresentation,
  fileType,
  fileSize,
  modifiedAt,
  materialization,
  canReveal,
  onReveal
}: {
  detailsOpen: boolean;
  source: ReturnType<typeof usePreviewExperience>["state"]["source"];
  metadata: ReturnType<typeof metadataFromSnapshot>;
  language: Parameters<typeof formatDate>[1];
  t: ReturnType<typeof useI18nContext>["t"];
  state: ReturnType<typeof usePreviewExperience>["state"];
  requestPreviewAsset: (request: PreviewAssetRequest) => Promise<import("../../../types/fileWorkspace").PreviewAssetArtifact>;
  updateNativePreviewGeometry: (previewId: string, presentation: PreviewNativePresentation) => Promise<PreviewSnapshot | null>;
  imagePresentation: ReturnType<typeof usePreviewImagePresentation>;
  pdfPresentation: ReturnType<typeof usePreviewPdfPresentation>;
  fileType: string | undefined;
  fileSize: number | undefined;
  modifiedAt: number | undefined;
  materialization: string | undefined;
  canReveal: boolean;
  onReveal: () => void;
}) {
  return (
    <div
      className="zc-quick-preview-body"
      data-preview-content="true"
      data-preview-details-open={detailsOpen ? "true" : "false"}
    >
      <div
        className="zc-quick-preview-content"
        data-preview-content-mode={state.phase === "content"
          ? state.snapshot?.representation?.representation.family ?? "state"
          : "state"}
      >
        {renderPreviewBody(
          state.phase,
          source,
          metadata,
          language,
          t,
          state.snapshot,
          requestPreviewAsset,
          updateNativePreviewGeometry,
          imagePresentation.publish,
          pdfPresentation.publish
        )}
      </div>
      {detailsOpen ? (
        <aside id="quick-preview-details" className="zc-quick-preview-inspector" aria-label={t("previewFileInfo")}>
          <h3>{t("previewFileInfo")}</h3>
          <dl data-preview-details-facts="true">
            {fileType ? <PreviewFact label={t("previewFileType")} value={fileType} /> : null}
            {fileSize === undefined ? null : <PreviewFact label={t("previewFileSize")} value={formatBytes(fileSize)} />}
            {modifiedAt === undefined ? null : <PreviewFact label={t("previewFileModified")} value={formatDate(String(modifiedAt), language)} />}
            {materialization ? <PreviewFact label={t("previewMaterializationLabel")} value={materialization} /> : null}
          </dl>
          {canReveal ? (
            <div className="zc-quick-preview-details-actions">
              <button type="button" className={buttonSecondary} onClick={onReveal} data-preview-reveal="true">
                {t("previewShowLocation")}
              </button>
            </div>
          ) : null}
        </aside>
      ) : null}
    </div>
  );
}

function PreviewFact({ label, value }: { label: string; value: string }) {
  return <div className="zc-quick-preview-fact"><dt>{label}</dt><dd title={value}>{value}</dd></div>;
}

/**
 * Type glyph for the preview identity block. Presentation only: the preview
 * source still comes from the controller's opaque projection.
 */
export function previewTypeIcon(
  extension: string | undefined,
  mediaType: string | undefined,
  entryKind: "file" | "directory" | undefined
) {
  if (entryKind === "directory") return Folder;
  const ext = (extension ?? "").toLocaleLowerCase();
  const type = (mediaType ?? "").toLocaleLowerCase();
  if (type.startsWith("image/") || ["png", "jpg", "jpeg", "gif", "webp", "svg", "heic", "avif"].includes(ext)) return ImageIcon;
  if (type.startsWith("video/") || ["mp4", "mov", "mkv", "webm", "avi"].includes(ext)) return Film;
  if (type.startsWith("audio/") || ["mp3", "wav", "flac", "m4a", "aac", "ogg"].includes(ext)) return Music;
  if (["zip", "gz", "tar", "7z", "rar", "dmg"].includes(ext)) return Archive;
  if (["xls", "xlsx", "csv", "tsv", "numbers"].includes(ext)) return Table;
  if (["pdf", "doc", "docx", "md", "txt", "rtf", "ppt", "pptx", "key"].includes(ext)) return FileText;
  return File;
}
