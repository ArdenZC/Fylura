import { usePane } from "../workbench/PaneContext";
import { AlertTriangle, FileText, Folder, FolderOpen, LoaderCircle } from "lucide-react";
import { memo, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Language } from "../../../i18n";
import type { FileWorkspaceController } from "../../../fileWorkspace";
import type { ThumbnailRequest, ThumbnailVariant } from "../../../types/fileWorkspace";
import type { Translator } from "../../../types/ui";
import { formatBytes, formatDate } from "../../../utils/format";
import { buttonSecondary, cn, virtualSpacer } from "../../../utils/tw";
import type {
  BrowsePresentationEntry,
  LibraryPresentationEntry,
  PresentationEntry
} from "../presentation/contracts";
import type { PresentationInteractionProjection, PresentationSelectionIntent } from "./interactionContracts";
import { selectionIntentFromModifiers } from "./interactionAdapters";
import { resolvePresentationContextMenuTarget } from "./contextMenuTarget";
import "./sharedFileGrid.css";

const CELL_MIN_WIDTH = 144;
const CELL_MAX_WIDTH = 176;
const GRID_GAP = 12;
const GRID_ROW_HEIGHT = 204;
const GRID_OVERSCAN = 4;
const LOAD_MORE_ROW_THRESHOLD = 2;
let nextThumbnailRequest = 0;

export type GridLoadMoreDecision =
  | { kind: "none" }
  | { kind: "clamp"; rowIndex: number }
  | { kind: "load" };

/**
 * Keeps an exact-count grid's logical scrollbar from becoming a paging
 * authority. A far jump is repositioned to the loaded source boundary; only
 * a demand inside the bounded near-end window may request one more page.
 */
export function decideGridLoadMore({
  source,
  hasMore,
  isLoadingMore,
  loadedRowCount,
  lastVisibleRow,
  columns,
  scrollTop
}: {
  source: "library" | "browse";
  hasMore: boolean;
  isLoadingMore: boolean;
  loadedRowCount: number;
  lastVisibleRow: number;
  columns: number;
  scrollTop: number;
}): GridLoadMoreDecision {
  if (!hasMore || isLoadingMore || loadedRowCount === 0 || lastVisibleRow < 0) return { kind: "none" };
  if (source === "browse" && scrollTop === 0) return { kind: "none" };

  const safeColumns = Math.max(1, columns);
  const loadedBoundaryRow = Math.max(0, Math.ceil(loadedRowCount / safeColumns) - 1);
  if (lastVisibleRow > loadedBoundaryRow + GRID_OVERSCAN) return { kind: "clamp", rowIndex: loadedBoundaryRow };
  if (lastVisibleRow < loadedBoundaryRow - LOAD_MORE_ROW_THRESHOLD) return { kind: "none" };
  return { kind: "load" };
}

/** Maps CSS cell geometry to the existing backend semantic variants. */
export function thumbnailVariantForCell(width: number, devicePixelRatio: number): ThumbnailVariant {
  const physicalWidth = Math.max(1, width) * Math.max(1, devicePixelRatio);
  if (physicalWidth <= 128) return "small";
  if (physicalWidth <= 320) return "medium";
  return "large";
}

function columnsForWidth(width: number) {
  if (width <= 0) return 1;
  const columns = Math.floor((width + GRID_GAP) / (CELL_MIN_WIDTH + GRID_GAP));
  return Math.max(1, Math.min(32, columns));
}

export function SharedFileGrid({
  interaction,
  language,
  t,
  controller,
  ariaLabel,
  emptyLabel,
  loadMoreLabel,
  loadingMoreLabel,
  onActivate,
  onEnter,
  onPreview,
  onContextMenu,
  onOpenContextMenu,
  onEscape
}: {
  interaction: PresentationInteractionProjection;
  language: Language;
  t: Translator;
  controller: FileWorkspaceController;
  ariaLabel: string;
  emptyLabel?: string;
  loadMoreLabel?: string;
  loadingMoreLabel?: string;
  onActivate?: (entry: PresentationEntry, trigger: HTMLElement) => void | Promise<void>;
  onEnter?: (entry: PresentationEntry, trigger: HTMLElement) => void | Promise<void>;
  onPreview?: (entry: PresentationEntry, trigger: HTMLElement, event: KeyboardEvent<HTMLDivElement>) => boolean | void;
  onContextMenu?: (event: MouseEvent<HTMLDivElement>, entry: PresentationEntry, index: number) => void;
  onOpenContextMenu?: (entry: PresentationEntry, index: number) => void;
  onEscape?: () => boolean;
}) {
  const pane = usePane();
  const gridRef = useRef<HTMLDivElement | null>(null);
  const loadMoreInFlightRef = useRef(false);
  const [width, setWidth] = useState(0);
  const columns = columnsForWidth(width);
  const cellWidth = width > 0
    ? Math.max(CELL_MIN_WIDTH, (width - GRID_GAP * (columns - 1)) / columns)
    : CELL_MIN_WIDTH;
  const rowCount = Math.ceil(interaction.rowCount / columns);
  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => gridRef.current,
    estimateSize: () => GRID_ROW_HEIGHT,
    overscan: GRID_OVERSCAN
  });
  const virtualRows = rowVirtualizer.getVirtualItems();
  const lastVisibleRow = virtualRows.at(-1)?.index ?? -1;
  const focusedRow = interaction.focusedIndex < 0 ? -1 : Math.floor(interaction.focusedIndex / columns);
  const focusedIsMounted = focusedRow >= 0 && virtualRows.some((row) => row.index === focusedRow);
  const focusedEntry = focusedIsMounted ? interaction.entryAt(interaction.focusedIndex) : undefined;

  useEffect(() => {
    const element = gridRef.current;
    if (!element) return;
    const updateWidth = () => setWidth(element.clientWidth);
    updateWidth();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const decision = decideGridLoadMore({
      source: interaction.source,
      hasMore: interaction.hasMore,
      isLoadingMore: interaction.isLoadingMore,
      loadedRowCount: interaction.loadedRowCount,
      lastVisibleRow,
      columns,
      scrollTop: gridRef.current?.scrollTop ?? 0
    });
    if (decision.kind === "clamp") {
      rowVirtualizer.scrollToIndex(decision.rowIndex, { align: "auto" });
      return;
    }
    if (decision.kind !== "load") return;
    if (loadMoreInFlightRef.current) return;
    loadMoreInFlightRef.current = true;
    Promise.resolve(interaction.actions.loadMore()).catch(() => undefined).finally(() => {
      loadMoreInFlightRef.current = false;
    });
  }, [
    columns,
    interaction.actions.loadMore,
    interaction.hasMore,
    interaction.isLoadingMore,
    interaction.loadedRowCount,
    interaction.source,
    lastVisibleRow,
    rowVirtualizer
  ]);

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.target !== event.currentTarget) return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
      event.preventDefault();
      interaction.actions.selectAll();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      if (!onEscape?.()) interaction.actions.clearSelection();
      return;
    }
    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
      event.preventDefault();
      const target = resolvePresentationContextMenuTarget(interaction);
      if (target) onOpenContextMenu?.(target.entry, target.index);
      return;
    }

    if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key)) {
      event.preventDefault();
      const nextIndex = nextGridIndex(event.key, interaction.focusedIndex, interaction.loadedRowCount, columns, gridRef.current);
      const entry = interaction.entryAt(nextIndex);
      if (!entry) return;
      if (event.shiftKey) selectGridEntry(interaction, entry, nextIndex, "range");
      else focusGridEntry(interaction, entry, nextIndex);
      rowVirtualizer.scrollToIndex(Math.floor(nextIndex / columns), {
        align: event.key === "PageUp" || event.key === "PageDown" ? "start" : "auto"
      });
      return;
    }

    if (event.key === "Enter") {
      const index = interaction.focusedIndex >= 0 ? interaction.focusedIndex : 0;
      const entry = interaction.entryAt(index);
      if (entry !== undefined && onEnter) {
        event.preventDefault();
        void onEnter(entry, gridRef.current ?? document.body);
      }
      return;
    }

    if (event.key === " " || event.key === "Space") {
      if (event.repeat || interaction.focusedIndex < 0) return;
      const index = interaction.focusedIndex;
      const entry = interaction.entryAt(index);
      const handled = entry !== undefined && onPreview?.(entry, gridRef.current ?? document.body, event) === true;
      if (handled) event.preventDefault();
    }
  }

  return (
    <div className="shared-file-grid-shell">
      <div
        ref={gridRef}
        className="shared-file-grid"
        role="grid"
        tabIndex={0}
        aria-label={ariaLabel}
        aria-multiselectable="true"
        aria-rowcount={interaction.source === "browse" && interaction.hasMore ? undefined : rowCount}
        aria-colcount={columns}
        aria-busy={interaction.isLoadingMore}
        aria-activedescendant={focusedEntry === undefined ? undefined : gridCellDomId(focusedEntry, pane?.id)}
        data-shared-file-grid="true"
        data-shared-file-grid-source={interaction.source}
        data-file-library-grid-columns={columns}
        data-file-library-grid-logical-count={interaction.rowCount}
        data-file-library-grid-loaded-count={interaction.loadedRowCount}
        data-file-library-grid-mounted-rows={virtualRows.length}
        data-file-library-grid-has-more={interaction.hasMore ? "true" : "false"}
        data-browse-grid={interaction.source === "browse" ? "true" : undefined}
        onKeyDown={handleKeyDown}
      >
        {interaction.rowCount === 0 ? (
          <div className="shared-file-grid-empty">{emptyLabel ?? t("browseUnknownValue")}</div>
        ) : (
          <div className={virtualSpacer} style={{ height: `${rowVirtualizer.getTotalSize()}px` }}>
            {virtualRows.map((virtualRow) => (
              <div
                key={virtualRow.key}
                className="shared-file-grid-row"
                role="row"
                aria-rowindex={virtualRow.index + 1}
                style={{
                  ...rowStyle(virtualRow.start, virtualRow.size),
                  gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`
                }}
              >
                {Array.from({ length: columns }, (_, columnIndex) => {
                  const index = virtualRow.index * columns + columnIndex;
                  const entry = index < interaction.rowCount ? interaction.entryAt(index) : undefined;
                  return entry === undefined ? (
                    <div key={`unloaded-${index}`} className="shared-file-grid-cell shared-file-grid-cell-unloaded" role="gridcell" aria-hidden="true" />
                  ) : (
                    <SharedFileGridCell
                      key={entry.renderKey}
                      entry={entry}
                      index={index}
                      selected={isGridEntrySelected(interaction, entry)}
                      focused={isGridEntryFocused(interaction, entry)}
                      cellWidth={cellWidth}
                      language={language}
                      t={t}
                      controller={controller}
                      columnIndex={columnIndex}
                      onClick={(event) => {
                        gridRef.current?.focus();
                        selectGridEntry(interaction, entry, index, selectionIntentFromModifiers(event));
                      }}
                      onDoubleClick={(event) => {
                        event.preventDefault();
                        if (onActivate) void onActivate(entry, event.currentTarget);
                      }}
                      onContextMenu={(event) => onContextMenu?.(event, entry, index)}
                      onActivate={() => {
                        if (onActivate) void onActivate(entry, gridRef.current ?? document.body);
                      }}
                    />
                  );
                })}
              </div>
            ))}
          </div>
        )}
      </div>
      {interaction.hasMore ? (
        <button
          className={cn(buttonSecondary, "shared-file-grid-load-more")}
          type="button"
          disabled={interaction.isLoadingMore}
          onClick={() => void interaction.actions.loadMore()}
        >
          {interaction.isLoadingMore
            ? loadingMoreLabel ?? t("browseEnumerationLoadingMore")
            : loadMoreLabel ?? t("browseLoadMore")}
        </button>
      ) : null}
    </div>
  );
}

const SharedFileGridCell = memo(function SharedFileGridCell({
  entry,
  index,
  selected,
  focused,
  cellWidth,
  language,
  t,
  controller,
  columnIndex,
  onClick,
  onDoubleClick,
  onContextMenu,
  onActivate
}: {
  entry: PresentationEntry;
  index: number;
  selected: boolean;
  focused: boolean;
  cellWidth: number;
  language: Language;
  t: Translator;
  controller: FileWorkspaceController;
  columnIndex: number;
  onClick: (event: MouseEvent<HTMLDivElement>) => void;
  onDoubleClick: (event: MouseEvent<HTMLDivElement>) => void;
  onContextMenu: (event: MouseEvent<HTMLDivElement>) => void;
  onActivate: () => void;
}) {
  const pane = usePane();
  const [thumbnail, setThumbnail] = useState<ThumbnailState>({ kind: "idle" });
  const devicePixelRatio = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
  const variant = thumbnailVariantForCell(cellWidth, devicePixelRatio);
  const sourceKey = presentationSourceKey(entry);
  const canRequest = (pane?.visible ?? true) && canRequestThumbnail(entry);

  useEffect(() => {
    let alive = true;
    let objectUrl: string | undefined;
    const requestId = `grid-thumbnail-${++nextThumbnailRequest}`;

    setThumbnail({ kind: canRequest ? "loading" : "unavailable" });
    const requestSource = thumbnailSourceFor(entry);
    if (!canRequest || requestSource === undefined) {
      return () => {
        alive = false;
      };
    }

    const request: ThumbnailRequest = {
      requestId,
      source: requestSource,
      variant,
      workClass: "interactive",
      ...(entry.source === "browse" ? { sessionId: entry.entryRef.browseSessionId } : {})
    };
    void controller.requestThumbnail(request).then((artifact) => {
      if (!alive || artifact === null || typeof URL === "undefined" || typeof URL.createObjectURL !== "function") {
        return;
      }
      const bytes = new ArrayBuffer(artifact.bytes.byteLength);
      new Uint8Array(bytes).set(artifact.bytes);
      const nextUrl = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
      if (!alive) {
        URL.revokeObjectURL(nextUrl);
        return;
      }
      objectUrl = nextUrl;
      setThumbnail({ kind: "ready", url: nextUrl });
    }).catch(() => {
      if (alive) setThumbnail({ kind: "unavailable" });
    });

    return () => {
      alive = false;
      if (objectUrl !== undefined && typeof URL !== "undefined" && typeof URL.revokeObjectURL === "function") {
        URL.revokeObjectURL(objectUrl);
      }
      void controller.cancelThumbnail(requestId).catch(() => undefined);
    };
  }, [canRequest, controller, entry.entryKind, entry.materialization, entry.source, entry.source === "library" ? entry.availability : entry.entryRef.browseSessionId, sourceKey, variant]);

  const isDirectory = entry.entryKind === "directory";
  const kindLabel = isDirectory ? t("browseFolder") : t("browseFile");
  const missingLabel = entry.source === "library" && entry.availability === "missing"
    ? t("libraryFileNotFound")
    : undefined;
  const details = [
    missingLabel,
    entry.typeHint,
    entry.size === undefined ? undefined : formatBytes(entry.size),
    entry.modifiedAt === undefined ? undefined : formatDate(String(entry.modifiedAt), language)
  ].filter(Boolean).join(" · ");
  const statusLabel = thumbnail.kind === "loading"
    ? t("fileLibraryThumbnailLoading")
    : thumbnail.kind === "unavailable"
      ? t("fileLibraryThumbnailUnavailable")
      : undefined;

  return (
    <div
      id={gridCellDomId(entry, pane?.id)}
      className={cn("shared-file-grid-cell", selected && "is-selected", focused && "is-focused", missingLabel && "is-missing")}
      draggable={selected}
      role="gridcell"
      tabIndex={-1}
      aria-selected={selected}
      aria-colindex={columnIndex + 1}
      aria-label={`${entry.displayName}, ${kindLabel}${statusLabel ? `, ${statusLabel}` : ""}`}
      data-grid-cell="true"
      data-grid-cell-index={index}
      data-grid-cell-status={isDirectory ? "directory" : thumbnail.kind}
      data-library-grid-row={entry.source === "library" ? entry.entryRef.fileId : undefined}
      data-browse-grid-entry={entry.source === "browse" ? "true" : undefined}
      data-browse-grid-entry-id={entry.source === "browse" ? entry.entryRef.entryId : undefined}
      data-browse-grid-entry-kind={entry.source === "browse" ? entry.entryKind : undefined}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      <div className="shared-file-grid-thumbnail" aria-hidden="true">
        {thumbnail.kind === "ready" ? <img src={thumbnail.url} alt="" /> : null}
        {thumbnail.kind === "loading" ? <LoaderCircle className="shared-file-grid-thumbnail-spinner" size={22} /> : null}
        {isDirectory ? <Folder size={32} strokeWidth={1.6} /> : thumbnail.kind !== "ready" ? <FileText size={30} strokeWidth={1.5} /> : null}
        {missingLabel ? <AlertTriangle className="shared-file-grid-warning-icon" size={14} /> : null}
        {isDirectory && entry.source === "browse" ? <FolderOpen className="shared-file-grid-folder-hint" size={14} /> : null}
      </div>
      <div className="shared-file-grid-name" title={entry.displayName}>{entry.displayName}</div>
      {details ? <div className={cn("shared-file-grid-details", missingLabel && "shared-file-grid-warning")} title={details}>{details}</div> : null}
    </div>
  );
});

type ThumbnailState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ready"; url: string }
  | { kind: "unavailable" };

function canRequestThumbnail(entry: PresentationEntry) {
  if (entry.entryKind === "directory") return false;
  if (entry.source === "library" && entry.availability !== "available") return false;
  return entry.materialization === undefined
    || entry.materialization === "local"
    || entry.materialization === "boundary_readable";
}

function thumbnailSourceFor(entry: PresentationEntry): ThumbnailRequest["source"] | undefined {
  if (!canRequestThumbnail(entry)) return undefined;
  return entry.entryRef;
}

function presentationSourceKey(entry: PresentationEntry) {
  return entry.source === "library"
    ? `managed:${entry.entryRef.fileId}`
    : `browse:${entry.entryRef.browseSessionId}:${entry.entryRef.entryId}`;
}

function selectGridEntry(
  interaction: PresentationInteractionProjection,
  entry: PresentationEntry,
  index: number,
  intent: PresentationSelectionIntent
) {
  if (interaction.source === "library" && entry.source === "library") interaction.actions.select(entry, index, intent);
  if (interaction.source === "browse" && entry.source === "browse") interaction.actions.select(entry, index, intent);
}

function focusGridEntry(interaction: PresentationInteractionProjection, entry: PresentationEntry, index: number) {
  if (interaction.source === "library" && entry.source === "library") interaction.actions.focus(entry, index);
  if (interaction.source === "browse" && entry.source === "browse") interaction.actions.focus(entry, index);
}

function isGridEntrySelected(interaction: PresentationInteractionProjection, entry: PresentationEntry) {
  if (interaction.source === "library" && entry.source === "library") return interaction.isSelected(entry);
  if (interaction.source === "browse" && entry.source === "browse") return interaction.isSelected(entry);
  return false;
}

function isGridEntryFocused(interaction: PresentationInteractionProjection, entry: PresentationEntry) {
  if (interaction.source === "library" && entry.source === "library") return interaction.isFocused(entry);
  if (interaction.source === "browse" && entry.source === "browse") return interaction.isFocused(entry);
  return false;
}

export function nextGridIndex(
  key: string,
  focusedIndex: number,
  loadedRowCount: number,
  columns: number,
  scrollElement: HTMLElement | null
) {
  if (loadedRowCount === 0) return -1;
  const safeColumns = Math.max(1, columns);
  const page = Math.max(1, Math.floor((scrollElement?.clientHeight ?? GRID_ROW_HEIGHT * 3) / GRID_ROW_HEIGHT) - 1) * safeColumns;
  if (focusedIndex < 0) {
    if (key === "End") return loadedRowCount - 1;
    return 0;
  }
  const nextIndex = key === "ArrowLeft"
    ? focusedIndex - 1
    : key === "ArrowRight"
      ? focusedIndex + 1
      : key === "ArrowUp"
        ? focusedIndex - safeColumns
        : key === "ArrowDown"
          ? focusedIndex + safeColumns
          : key === "PageUp"
            ? focusedIndex - page
            : key === "PageDown"
              ? focusedIndex + page
              : key === "Home"
                ? Math.floor(focusedIndex / safeColumns) * safeColumns
                : key === "End"
                  ? Math.min(loadedRowCount - 1, Math.floor(focusedIndex / safeColumns) * safeColumns + safeColumns - 1)
                  : focusedIndex;
  return Math.max(0, Math.min(loadedRowCount - 1, nextIndex));
}

function rowStyle(start: number, size: number): CSSProperties {
  return { height: `${size}px`, transform: `translateY(${start}px)` };
}

function gridCellDomId(entry: PresentationEntry, paneId?: string) {
  const prefix = paneId ? `${paneId}-` : "";
  return entry.source === "library"
    ? `${prefix}library-grid-cell-${entry.entryRef.fileId}`
    : `${prefix}browse-grid-cell-${entry.entryRef.entryId}`;
}
