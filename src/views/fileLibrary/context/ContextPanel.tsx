import { SELECTED_PREVIEW_LIMIT } from "../preview/useSelectedPreview";
import { useState } from "react";
import { buttonSecondary } from "../../../utils/tw";
import type { Translator } from "../../../types/ui";
import { File, Folder, Info, X } from "lucide-react";
import { SideSheet } from "../../shared/ui";
import { FileLibraryInspector } from "../../vault/components/FileLibraryInspector";
import { formatBytes, formatDate } from "../../../utils/format";
import { cn } from "../../../utils/tw";
import { useContextPanelPresentation } from "./contextPanelPresentation";
import { restoreFileLibraryFocus } from "../fileLibraryInteraction";
import {
  browsePresentationEntryLabel,
  browseSelectedSummaryText,
  type BrowseContextProjection,
  type ContextPanelProjection
} from "./contextPanelProjection";

export function ContextPanel({
  projection,
  open,
  onClose,
  restoreFocus
}: {
  projection: ContextPanelProjection;
  open: boolean;
  onClose: () => void;
  restoreFocus: () => HTMLElement | null;
}) {
  const layout = useContextPanelPresentation();
  if (!open || projection.kind === "none") return null;

  const title = contextTitle(projection);
  const description = contextDescription(projection);
  const closeLabel = contextCloseLabel(projection);
  const closePanel = () => {
    onClose();
    if (layout === "large") restoreFileLibraryFocus(restoreFocus);
  };

  const content = (
    <div className="file-library-context-panel-content" data-file-library-context-content={projection.kind}>
      <SelectedPreviewActions key={projection.source === "library" ? projection.inspector.selectedFiles.slice(0, SELECTED_PREVIEW_LIMIT).map((file) => file.id).join("|") : projection.selectedEntries.slice(0, SELECTED_PREVIEW_LIMIT).map((entry) => entry.entryRef.entryId).join("|")} projection={projection} />
      {projection.source === "library"
        ? <FileLibraryInspector {...projection.inspector} />
        : <BrowseContextContent projection={projection} />}
    </div>
  );

  if (layout === "large") {
    return (
      <aside
        className={cn("file-library-context-panel", "file-library-context-panel-inline")}
        aria-label={title}
        onKeyDown={(event) => { if (event.key === "Escape") { event.stopPropagation(); closePanel(); } }}
        data-file-library-context-panel="true"
        data-file-library-context-source={projection.source}
        data-file-library-context-layout="inline"
      >
        <header className="file-library-context-inline-header">
          <h2 className="text-sm font-semibold text-[var(--zc-text-primary)]">{title}</h2>
          <button
            type="button"
            className="file-library-context-close"
            aria-label={closeLabel}
            title={closeLabel}
            onClick={closePanel}
          >
            <X size={16} aria-hidden="true" />
          </button>
        </header>
        {content}
      </aside>
    );
  }

  return (
    <SideSheet
      open
      title={title}
      description={description}
      onClose={closePanel}
      closeLabel={closeLabel}
      modalId="file-library-context-panel"
      restoreFocus={restoreFocus}
    >
      <div
        className="file-library-context-panel"
        data-file-library-context-panel="true"
        data-file-library-context-source={projection.source}
        data-file-library-context-layout="overlay"
      >
        {content}
      </div>
    </SideSheet>
  );
}

function BrowseContextContent({ projection }: { projection: BrowseContextProjection }) {
  const { t } = projection;
  const selected = projection.selectedEntries[0];
  if (projection.kind === "inspector" && selected) {
    const Icon = selected.entryKind === "directory" ? Folder : File;
    return (
      <section className="file-library-context-content-inner" aria-labelledby="file-library-context-entry-title">
        <div className="file-library-context-entry-heading">
          <span className="file-library-context-entry-icon" aria-hidden="true"><Icon size={18} /></span>
          <div className="min-w-0">
            <h2 id="file-library-context-entry-title" className="break-words text-base font-semibold text-[var(--zc-text-primary)]">{selected.displayName}</h2>
            <p className="mt-1 text-xs text-[var(--zc-text-secondary)]">{projection.locationLabel}</p>
          </div>
        </div>
        <dl className="file-library-context-facts">
          <BrowseFact label={t("fileLibraryContextType")} value={browsePresentationEntryLabel(selected)} />
          <BrowseFact label={t("fileLibraryContextKind")} value={selected.entryKind === "directory" ? t("browseFolder") : t("browseFile")} />
          <BrowseFact label={t("fileLibraryContextSize")} value={selected.size === undefined ? t("browseUnknownValue") : formatBytes(selected.size)} />
          <BrowseFact label={t("fileLibraryContextModified")} value={selected.modifiedAt === undefined ? t("browseUnknownValue") : formatDate(String(selected.modifiedAt), projection.language)} />
          {selected.createdAt === undefined ? null : <BrowseFact label={t("fileLibraryContextCreated")} value={formatDate(String(selected.createdAt), projection.language)} />}
          <BrowseFact label={t("fileLibraryContextMaterialization")} value={materializationLabel(selected.materialization, t)} />
        </dl>
        <p className="file-library-context-note"><Info size={15} aria-hidden="true" />{t("fileLibraryContextBrowseSafety")}</p>
        <p className="file-library-context-note">{t("u4BrowseUnavailable")}</p>
      </section>
    );
  }

  return (
    <section className="file-library-context-content-inner" aria-labelledby="file-library-context-selection-title">
      <div className="file-library-context-entry-heading">
        <span className="file-library-context-entry-icon" aria-hidden="true"><Info size={18} /></span>
        <div className="min-w-0">
          <h2 id="file-library-context-selection-title" className="text-base font-semibold text-[var(--zc-text-primary)]">{t("fileLibraryContextSelectionTitle")}</h2>
          <p className="mt-1 text-xs text-[var(--zc-text-secondary)]">{projection.locationLabel}</p>
        </div>
      </div>
      <p className="file-library-context-selection-count">{browseSelectedSummaryText(projection)}</p>
      <dl className="file-library-context-facts">
        <BrowseFact label={t("fileLibraryContextSize")} value={browseSizeLabel(projection)} />
        <BrowseFact label={t("fileLibraryContextTypes")} value={projection.typeCounts.map((item) => `${item.label} ×${item.count}`).join(" · ") || t("browseUnknownValue")} />
      </dl>
      <p className="file-library-context-note"><Info size={15} aria-hidden="true" />{t("fileLibraryContextBrowseMultiSafety")}</p>
      <p className="file-library-context-note">{t("u4BrowseUnavailable")}</p>
    </section>
  );
}

function BrowseFact({ label, value }: { label: string; value: string }) {
  return <div><dt>{label}</dt><dd title={value}>{value}</dd></div>;
}

function browseSizeLabel(projection: BrowseContextProjection) {
  if (projection.size.total === null) return projection.t("fileLibraryContextUnknownSize");
  const formatted = formatBytes(projection.size.total);
  return projection.size.state === "partial"
    ? `${formatted} · ${projection.t("fileLibraryContextPartialSize")}`
    : formatted;
}

function materializationLabel(value: string | undefined, t: BrowseContextProjection["t"]) {
  if (value === "metadata_only") return t("browseMaterializationMetadata");
  if (value === "remote_placeholder") return t("browseMaterializationRemote");
  if (value === "hydrating") return t("browseMaterializationHydrating");
  if (value === "unavailable") return t("browseMaterializationUnavailable");
  if (value === undefined || value === "unknown") return t("browseMaterializationUnknown");
  return t("fileLibraryContextLocal");
}

function contextTitle(projection: ContextPanelProjection) {
  return projection.source === "library"
    ? projection.inspector.t("fileLibraryContextTitle")
    : projection.t("fileLibraryContextTitle");
}

function contextDescription(projection: ContextPanelProjection) {
  return projection.source === "library"
    ? projection.inspector.t("fileLibraryContextDescription")
    : projection.t("fileLibraryContextDescription");
}

function contextCloseLabel(projection: ContextPanelProjection) {
  return projection.source === "library"
    ? projection.inspector.t("fileLibraryContextClose")
    : projection.t("fileLibraryContextClose");
}

function SelectedPreviewActions({ projection }: { projection: ContextPanelProjection }) {
  const [index, setIndex] = useState(0);
  const t: Translator = projection.source === "library" ? projection.inspector.t : projection.t;
  const items = projection.source === "library" ? projection.inspector.selectedFiles.slice(0, SELECTED_PREVIEW_LIMIT) : projection.selectedEntries.slice(0, SELECTED_PREVIEW_LIMIT);
  const current = items[Math.min(index, Math.max(0, items.length - 1))];
  if (!current) return null;
  const name = "name" in current ? current.name : current.displayName;
  const preview = (position: number, trigger: HTMLButtonElement) => {
    setIndex(position);
    if (projection.source === "library") {
      const file = projection.inspector.selectedFiles[position];
      if (file && !file.isStale) projection.inspector.onPreviewSelection?.(file.id, trigger);
    } else {
      const entry = projection.selectedEntries[position];
      if (entry) projection.onPreview?.(entry, trigger);
    }
  };
  return <section className="grid gap-2 border-b border-[var(--zc-divider)] p-3" aria-label={t("u4SelectionPreview")}>
    <p className="break-words text-xs">{name} · {t("u4SelectionPreview")} {index + 1}/{items.length}</p>
    <div className="flex flex-wrap gap-2">
      <button className={buttonSecondary} disabled={index === 0} aria-label={t("u4Previous")} onClick={(event) => preview(index - 1, event.currentTarget)}>←</button>
      <button className={buttonSecondary} disabled={"isStale" in current && current.isStale} onClick={(event) => preview(index, event.currentTarget)}>{t("libraryPreview")}</button>
      <button className={buttonSecondary} disabled={index + 1 >= items.length} aria-label={t("u4Next")} onClick={(event) => preview(index + 1, event.currentTarget)}>→</button>
    </div>
  </section>;
}
