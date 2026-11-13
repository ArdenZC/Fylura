import { usePane } from "../workbench/PaneContext";
import { AlertTriangle, FileText, Folder, FolderOpen } from "lucide-react";
import { memo, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { BrowseSortSpec } from "../../../types/fileWorkspace";
import type { Language } from "../../../i18n";
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
import { useBrowseMarqueeSelection } from "./useBrowseMarqueeSelection";
import "./sharedFileList.css";

const ROW_HEIGHT = 44;
const OVERSCAN = 8;
const LOAD_MORE_THRESHOLD = 4;
const LIST_COLUMN_MIN_WIDTHS = [180, 80, 100, 100, 80] as const;

function readListColumnWidths() {
  try {
    const parsed = JSON.parse(window.localStorage.getItem("fylura.files.listColumnWidths") ?? "null");
    if (Array.isArray(parsed) && parsed.length === LIST_COLUMN_MIN_WIDTHS.length && parsed.every((value, index) => Number.isFinite(value) && value >= LIST_COLUMN_MIN_WIDTHS[index]! && value <= 1200)) {
      return parsed as number[];
    }
  } catch { /* Use the default list geometry when storage is unavailable. */ }
  return [290, 110, 110, 120, 85];
}

export type ListLoadMoreDecision =
  | { kind: "none" }
  | { kind: "clamp"; rowIndex: number }
  | { kind: "load" };

/**
 * Keeps an exact-count list's logical scrollbar from becoming a paging
 * authority. A far jump is repositioned to the loaded source boundary; only
 * a demand inside the bounded near-end window may request one more page.
 */
export function decideListLoadMore({
  source,
  hasMore,
  isLoadingMore,
  loadedRowCount,
  lastVisibleIndex,
  scrollTop
}: {
  source: "library" | "browse";
  hasMore: boolean;
  isLoadingMore: boolean;
  loadedRowCount: number;
  lastVisibleIndex: number;
  scrollTop: number;
}): ListLoadMoreDecision {
  if (!hasMore || isLoadingMore || loadedRowCount === 0 || lastVisibleIndex < 0) return { kind: "none" };
  if (source === "browse" && scrollTop === 0) return { kind: "none" };

  const loadedBoundaryIndex = loadedRowCount - 1;
  if (lastVisibleIndex > loadedBoundaryIndex + OVERSCAN) {
    return { kind: "clamp", rowIndex: loadedBoundaryIndex };
  }
  if (lastVisibleIndex < loadedBoundaryIndex - LOAD_MORE_THRESHOLD) return { kind: "none" };
  return { kind: "load" };
}

export function SharedFileList({
  interaction,
  language,
  t,
  ariaLabel,
  emptyLabel,
  loadMoreLabel,
  loadingMoreLabel,
  loadedAllLabel,
  onActivate,
  onEnter,
  onPreview,
  onContextMenu,
  onOpenContextMenu,
  onEscape,
  onBackgroundContextMenu,
  onNavigateUp,
  onRefresh,
  onTypeToSelect,
  sort,
  onSort
}: {
  interaction: PresentationInteractionProjection;
  language: Language;
  t: Translator;
  ariaLabel: string;
  emptyLabel?: string;
  loadMoreLabel?: string;
  loadingMoreLabel?: string;
  loadedAllLabel?: string;
  onActivate?: (entry: PresentationEntry, trigger: HTMLElement) => void | Promise<void>;
  onEnter?: (entry: PresentationEntry, trigger: HTMLElement) => void | Promise<void>;
  onPreview?: (entry: PresentationEntry, trigger: HTMLElement, event: KeyboardEvent<HTMLDivElement>) => boolean | void;
  onContextMenu?: (
    event: MouseEvent<HTMLDivElement>,
    entry: PresentationEntry,
    index: number
  ) => void;
  onOpenContextMenu?: (entry: PresentationEntry, index: number) => void;
  onEscape?: () => boolean;
  onBackgroundContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
  onNavigateUp?: () => void;
  onRefresh?: () => void;
  onTypeToSelect?: (text: string) => void;
  sort?: BrowseSortSpec;
  onSort?: (field: BrowseSortSpec["field"]) => void;
}) {
  const pane = usePane();
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const resizeRef = useRef<{ index: number; startX: number; initialWidth: number } | null>(null);
  const [columnWidths, setColumnWidths] = useState(readListColumnWidths);
  const columnWidthsRef = useRef(columnWidths);
  columnWidthsRef.current = columnWidths;
  const marquee = useBrowseMarqueeSelection(scrollRef, interaction);
  const loadMoreInFlightRef = useRef(false);
  const typeAheadRef = useRef({ text: "", time: 0 });
  const rowVirtualizer = useVirtualizer({
    count: interaction.rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: OVERSCAN
  });
  const virtualItems = rowVirtualizer.getVirtualItems();
  const lastVisibleIndex = virtualItems.at(-1)?.index ?? -1;
  const focusedIsMounted = interaction.focusedIndex >= 0
    && virtualItems.some((item) => item.index === interaction.focusedIndex);
  const activeDescendant = focusedIsMounted
    ? interaction.entryAt(interaction.focusedIndex)
    : undefined;

  // Comparison can focus an already loaded row outside the current viewport.
  useEffect(() => {
    if (interaction.source === "browse" && interaction.focusedIndex >= 0) rowVirtualizer.scrollToIndex(interaction.focusedIndex, { align: "auto" });
  }, [interaction.source, interaction.focusedIndex, rowVirtualizer]);

  useEffect(() => {
    const decision = decideListLoadMore({
      source: interaction.source,
      hasMore: interaction.hasMore,
      isLoadingMore: interaction.isLoadingMore,
      loadedRowCount: interaction.loadedRowCount,
      lastVisibleIndex,
      scrollTop: scrollRef.current?.scrollTop ?? 0
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
    interaction.actions.loadMore,
    interaction.hasMore,
    interaction.isLoadingMore,
    interaction.loadedRowCount,
    interaction.source,
    lastVisibleIndex,
    rowVirtualizer
  ]);

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.target !== event.currentTarget) return;

    if ((event.altKey && event.key === "ArrowUp") || event.key === "Backspace") {
      if (onNavigateUp) {
        event.preventDefault();
        onNavigateUp();
      }
      return;
    }
    if (event.key === "F5") {
      if (onRefresh) {
        event.preventDefault();
        onRefresh();
      }
      return;
    }
    if (interaction.source === "browse" && !event.nativeEvent.isComposing && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && event.key.length === 1 && !/\s/u.test(event.key)) {
      const now = Date.now();
      const prefix = now - typeAheadRef.current.time < 700 ? typeAheadRef.current.text + event.key : event.key;
      typeAheadRef.current = { text: prefix, time: now };
      onTypeToSelect?.(prefix);
      return;
    }

    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
      event.preventDefault();
      interaction.actions.selectAll();
      return;
    }
    if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === "i") {
      if (interaction.actions.invertSelection) {
        event.preventDefault();
        interaction.actions.invertSelection();
      }
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

    const navigationKey = ["ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"].includes(event.key);
    if (navigationKey) {
      event.preventDefault();
      const nextIndex = nextNavigationIndex(event.key, interaction.focusedIndex, interaction.loadedRowCount, scrollRef.current);
      const entry = interaction.entryAt(nextIndex);
      if (!entry) return;
      if (event.shiftKey) selectInteractionEntry(interaction, entry, nextIndex, "range");
      else focusInteractionEntry(interaction, entry, nextIndex);
      rowVirtualizer.scrollToIndex(nextIndex, {
        align: event.key === "PageUp" || event.key === "PageDown" ? "start" : "auto"
      });
      return;
    }

    if (event.key === "Enter") {
      const index = interaction.focusedIndex >= 0 ? interaction.focusedIndex : 0;
      const entry = interaction.entryAt(index);
      if (entry !== undefined && onEnter) {
        event.preventDefault();
        void onEnter(entry, scrollRef.current ?? document.body);
      }
      return;
    }

    if (event.key === " " || event.key === "Space") {
      if (event.repeat || interaction.focusedIndex < 0) return;
      const index = interaction.focusedIndex;
      const entry = interaction.entryAt(index);
      const handled = entry !== undefined && onPreview?.(entry, scrollRef.current ?? document.body, event) === true;
      if (handled) event.preventDefault();
    }
  }

  const showCreated = interaction.source === "browse";
  const columns: { field: BrowseSortSpec["field"]; label: string }[] = [
    { field: "name", label: t("fileName") },
    { field: "type", label: t("fileType") },
    ...(showCreated ? [{ field: "created" as const, label: t("u3Created") }] : []),
    { field: "modified", label: t("fileModified") },
    { field: "size", label: t("fileSize") }
  ];
  const widthIndexes = showCreated ? [0, 1, 2, 3, 4] : [0, 1, 3, 4];
  const columnsStyle = widthIndexes.map((index) => `${columnWidths[index]}px`).join(" ");
  function resizePointerDown(event: ReactPointerEvent<HTMLSpanElement>, widthIndex: number) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    resizeRef.current = { index: widthIndex, startX: event.clientX, initialWidth: columnWidths[widthIndex]! };
    event.currentTarget.setPointerCapture(event.pointerId);
  }
  function resizePointerMove(event: ReactPointerEvent<HTMLSpanElement>) {
    const resize = resizeRef.current;
    if (!resize) return;
    const width = Math.min(1200, Math.max(LIST_COLUMN_MIN_WIDTHS[resize.index]!, resize.initialWidth + event.clientX - resize.startX));
    setColumnWidths((current) => current.map((value, index) => index === resize.index ? width : value));
  }
  function resizePointerUp() {
    if (!resizeRef.current) return;
    resizeRef.current = null;
    try { window.localStorage.setItem("fylura.files.listColumnWidths", JSON.stringify(columnWidthsRef.current)); } catch { /* Width changes remain active for this session. */ }
  }
  function resizeByKeyboard(event: KeyboardEvent<HTMLSpanElement>, widthIndex: number) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    event.stopPropagation();
    const width = Math.min(1200, Math.max(LIST_COLUMN_MIN_WIDTHS[widthIndex]!, columnWidths[widthIndex]! + (event.key === "ArrowRight" ? 16 : -16)));
    setColumnWidths((current) => current.map((value, index) => index === widthIndex ? width : value));
    const next = columnWidths.map((value, index) => index === widthIndex ? width : value);
    try { window.localStorage.setItem("fylura.files.listColumnWidths", JSON.stringify(next)); } catch { /* Width changes remain active for this session. */ }
  }
  return (
    <div className={cn("shared-file-list-shell", showCreated && "has-created-column")} style={{ "--shared-file-list-columns": columnsStyle } as CSSProperties}>
      <div className="shared-file-list-header" role="group" aria-label={t("u3Sort")}>
        {columns.map(({ field, label }, index) => {
          const widthIndex = widthIndexes[index]!;
          return <div key={field} className="shared-file-list-header-cell">
            {onSort ? <button type="button" data-browse-sort-column={field}
              className={field === "size" ? "shared-file-list-size-heading" : undefined}
              aria-pressed={sort?.field === field}
              aria-label={sort?.field === field ? `${label}, ${sort.direction === "asc" ? t("u3Asc") : t("u3Desc")}` : label}
              onClick={() => onSort(field)}>
              {label}{sort?.field === field ? <span aria-hidden="true">{sort.direction === "asc" ? " ↑" : " ↓"}</span> : null}
            </button> : <span className={field === "size" ? "shared-file-list-size-heading" : undefined}>{label}</span>}
            {index < columns.length - 1 ? <span role="separator" aria-orientation="vertical" aria-label={`${label} column width`} aria-valuemin={LIST_COLUMN_MIN_WIDTHS[widthIndex]} aria-valuemax={1200} aria-valuenow={columnWidths[widthIndex]} tabIndex={0} className="shared-file-list-column-resize"
              onPointerDown={(event) => resizePointerDown(event, widthIndex)} onPointerMove={resizePointerMove} onPointerUp={resizePointerUp}
              onPointerCancel={resizePointerUp} onKeyDown={(event) => resizeByKeyboard(event, widthIndex)} /> : null}
          </div>;
        })}
      </div>
      <div
        ref={scrollRef}
        className="shared-file-list"
        role="listbox"
        tabIndex={0}
        aria-label={ariaLabel}
        aria-multiselectable="true"
        aria-busy={interaction.isLoadingMore}
        aria-activedescendant={activeDescendant === undefined ? undefined : rowDomId(activeDescendant, pane?.id)}
        data-shared-file-list="true"
        data-shared-file-list-source={interaction.source}
        data-file-library-scroll-owner="tanstack-virtualizer"
        data-file-library-logical-count={interaction.rowCount}
        data-file-library-has-more={interaction.hasMore ? "true" : "false"}
        data-browse-list={interaction.source === "browse" ? "true" : undefined}
        data-browse-logical-count={interaction.source === "browse" ? interaction.rowCount : undefined}
        onKeyDown={handleKeyDown}
        onPointerDown={marquee.onPointerDown}
        onPointerMove={marquee.onPointerMove}
        onPointerUp={marquee.onPointerUp}
        onPointerCancel={marquee.onPointerCancel}
        onContextMenu={(event) => {
          if (!(event.target instanceof Element) || !event.target.closest("[data-file-entry-kind]")) onBackgroundContextMenu?.(event);
        }}
      >
        {marquee.rectangle ? <div className="file-selection-marquee" aria-hidden="true" style={{
          left: marquee.rectangle.left,
          top: marquee.rectangle.top,
          width: marquee.rectangle.right - marquee.rectangle.left,
          height: marquee.rectangle.bottom - marquee.rectangle.top
        }} /> : null}
        {interaction.rowCount === 0 ? (
          <div className="shared-file-list-empty">{emptyLabel ?? t("browseUnknownValue")}</div>
        ) : (
          <div className={virtualSpacer} style={{ height: `${rowVirtualizer.getTotalSize()}px` }}>
            {virtualItems.map((virtualRow) => {
              const entry = interaction.entryAt(virtualRow.index);
              if (entry === undefined) {
                return <div key={`unloaded-${virtualRow.index}`} className="shared-file-list-row shared-file-list-row-unloaded" style={rowStyle(virtualRow.start, virtualRow.size)} aria-hidden="true" />;
              }
              return (
                <SharedFileRow
                  key={entry.renderKey}
                  entry={entry}
                  showCreated={showCreated}
                  index={virtualRow.index}
                  selected={isInteractionEntrySelected(interaction, entry)}
                  focused={isInteractionEntryFocused(interaction, entry)}
                  language={language}
                  t={t}
                  style={rowStyle(virtualRow.start, virtualRow.size)}
                  onClick={(event) => {
                    scrollRef.current?.focus();
                    selectInteractionEntry(interaction, entry, virtualRow.index, selectionIntentFromModifiers(event));
                  }}
                  onDoubleClick={(event) => {
                    event.preventDefault();
                    if (onActivate) void onActivate(entry, event.currentTarget);
                  }}
                  onContextMenu={(event) => onContextMenu?.(event, entry, virtualRow.index)}
                  onActivate={() => {
                    if (onActivate) void onActivate(entry, scrollRef.current ?? document.body);
                  }}
                />
              );
            })}
          </div>
        )}
      </div>
      {interaction.hasMore ? (
        <button
          className={cn(buttonSecondary, "shared-file-list-load-more")}
          type="button"
          disabled={interaction.isLoadingMore}
          onClick={() => void interaction.actions.loadMore()}
        >
          {interaction.isLoadingMore
            ? loadingMoreLabel ?? t("browseEnumerationLoadingMore")
            : loadMoreLabel ?? t("browseLoadMore")}
        </button>
      ) : loadedAllLabel && interaction.loadedRowCount > 0 ? (
        <p className="shared-file-list-loaded-all">{loadedAllLabel}</p>
      ) : null}
    </div>
  );
}

const SharedFileRow = memo(function SharedFileRow({
  entry,
  showCreated,
  index,
  selected,
  focused,
  language,
  t,
  style,
  onClick,
  onDoubleClick,
  onContextMenu,
  onActivate
}: {
  entry: PresentationEntry;
  showCreated: boolean;
  index: number;
  selected: boolean;
  focused: boolean;
  language: Language;
  t: Translator;
  style: React.CSSProperties;
  onClick: (event: MouseEvent<HTMLDivElement>) => void;
  onDoubleClick: (event: MouseEvent<HTMLDivElement>) => void;
  onContextMenu: (event: MouseEvent<HTMLDivElement>) => void;
  onActivate: () => void;
}) {
  const pane = usePane();
  const isDirectory = entry.entryKind === "directory";
  const kindLabel = isDirectory ? t("browseFolder") : t("browseFile");
  const missingLabel = entry.source === "library" && entry.availability === "missing"
    ? t("libraryFileNotFound")
    : undefined;
  const details = [
    missingLabel,
    entry.typeHint,
    entry.materialization === undefined || entry.source === "browse" && entry.materialization === "unknown" ? undefined : materializationLabel(entry.materialization, t)
  ].filter(Boolean).join(" · ");

  return (
    <div
      id={rowDomId(entry, pane?.id)}
      className={cn(
        "shared-file-list-row",
        selected && "is-selected",
        focused && "is-focused",
        missingLabel && "is-missing"
      )}
      style={style}
      data-file-entry-kind={entry.entryKind}
      data-file-entry-name={entry.displayName}
      role="option"
      tabIndex={-1}
      aria-selected={selected}
      aria-label={`${entry.displayName}, ${kindLabel}${missingLabel ? `, ${missingLabel}` : ""}`}
      data-virtual-row-index={index}
      data-library-row={entry.source === "library" ? entry.entryRef.fileId : undefined}
      data-library-availability={entry.source === "library" ? entry.availability : undefined}
      data-browse-entry={entry.source === "browse" ? "true" : undefined}
      data-browse-session-id={entry.source === "browse" ? entry.entryRef.browseSessionId : undefined}
      data-browse-entry-id={entry.source === "browse" ? entry.entryRef.entryId : undefined}
      data-browse-entry-kind={entry.source === "browse" ? entry.entryKind : undefined}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      <span className="shared-file-list-name">
        <span className="shared-file-list-icon" aria-hidden="true">
          {isDirectory ? <Folder size={17} /> : <FileText size={17} />}
          {missingLabel ? <AlertTriangle className="shared-file-list-warning-icon" size={13} /> : null}
        </span>
        <span className="shared-file-list-name-copy">
          <strong title={entry.displayName}>{entry.displayName}</strong>
          {details ? <span className={cn(missingLabel && "shared-file-list-warning")} title={details}>{details}</span> : null}
        </span>
      </span>
      <span className="shared-file-list-kind">{isDirectory ? kindLabel : entry.typeHint ?? entry.extension ?? kindLabel}</span>
      {showCreated ? <time className="shared-file-list-created">
        {entry.createdAt === undefined ? t("browseUnknownValue") : formatDate(String(entry.createdAt), language)}
      </time> : null}
      <time className="shared-file-list-modified" dateTime={entry.modifiedAt === undefined ? undefined : String(entry.modifiedAt)}>
        {entry.modifiedAt === undefined ? t("browseUnknownValue") : formatDate(String(entry.modifiedAt), language)}
      </time>
      <span className="shared-file-list-size">
        {entry.size === undefined ? t("browseUnknownValue") : formatBytes(entry.size)}
      </span>
      {entry.source === "browse" && isDirectory ? (
        <button
          className="shared-file-list-open-folder"
          type="button"
          aria-label={`${t("browseOpenFolder")}: ${entry.displayName}`}
          onClick={(event) => {
            event.stopPropagation();
            onActivate();
          }}
        >
          <FolderOpen size={15} aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
});

function selectInteractionEntry(
  interaction: PresentationInteractionProjection,
  entry: PresentationEntry,
  index: number,
  intent: PresentationSelectionIntent
) {
  if (interaction.source === "library" && entry.source === "library") {
    interaction.actions.select(entry, index, intent);
  } else if (interaction.source === "browse" && entry.source === "browse") {
    interaction.actions.select(entry, index, intent);
  }
}

function focusInteractionEntry(
  interaction: PresentationInteractionProjection,
  entry: PresentationEntry,
  index: number
) {
  if (interaction.source === "library" && entry.source === "library") {
    interaction.actions.focus(entry, index);
  } else if (interaction.source === "browse" && entry.source === "browse") {
    interaction.actions.focus(entry, index);
  }
}

function isInteractionEntrySelected(
  interaction: PresentationInteractionProjection,
  entry: PresentationEntry
) {
  if (interaction.source === "library" && entry.source === "library") return interaction.isSelected(entry);
  if (interaction.source === "browse" && entry.source === "browse") return interaction.isSelected(entry);
  return false;
}

function isInteractionEntryFocused(
  interaction: PresentationInteractionProjection,
  entry: PresentationEntry
) {
  if (interaction.source === "library" && entry.source === "library") return interaction.isFocused(entry);
  if (interaction.source === "browse" && entry.source === "browse") return interaction.isFocused(entry);
  return false;
}

export function nextNavigationIndex(
  key: string,
  focusedIndex: number,
  loadedRowCount: number,
  scrollElement: HTMLElement | null
) {
  if (loadedRowCount === 0) return -1;
  const page = Math.max(1, Math.floor((scrollElement?.clientHeight ?? ROW_HEIGHT * 8) / ROW_HEIGHT) - 1);
  if (focusedIndex < 0) {
    switch (key) {
      case "ArrowDown":
      case "ArrowUp":
      case "Home":
      case "PageUp":
        return 0;
      case "End":
        return loadedRowCount - 1;
      case "PageDown":
        return Math.min(loadedRowCount - 1, page);
      default:
        return 0;
    }
  }

  const nextIndex = key === "ArrowUp"
    ? focusedIndex - 1
    : key === "ArrowDown"
      ? focusedIndex + 1
      : key === "PageUp"
        ? focusedIndex - page
        : key === "PageDown"
          ? focusedIndex + page
          : key === "Home"
            ? 0
            : key === "End"
              ? loadedRowCount - 1
              : focusedIndex;
  return Math.max(0, Math.min(loadedRowCount - 1, nextIndex));
}

function rowStyle(start: number, size: number): React.CSSProperties {
  return { height: `${size}px`, transform: `translateY(${start}px)` };
}

function rowDomId(entry: PresentationEntry, paneId?: string) {
  const prefix = paneId ? `${paneId}-` : "";
  return entry.source === "library"
    ? `${prefix}library-row-${entry.entryRef.fileId}`
    : `${prefix}browse-row-${entry.entryRef.entryId}`;
}

function materializationLabel(
  state: NonNullable<LibraryPresentationEntry["materialization"] | BrowsePresentationEntry["materialization"]>,
  t: Translator
) {
  switch (state) {
    case "metadata_only": return t("browseMaterializationMetadata");
    case "remote_placeholder": return t("browseMaterializationRemote");
    case "hydrating": return t("browseMaterializationHydrating");
    case "unavailable": return t("browseMaterializationUnavailable");
    case "unknown": return t("browseMaterializationUnknown");
    default: return undefined;
  }
}
