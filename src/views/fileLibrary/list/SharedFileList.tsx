import { usePane } from "../workbench/PaneContext";
import { useAppStore } from "../../../store/useAppStore";
import { AlertTriangle, Check, FileText, Folder, FolderOpen } from "lucide-react";
import { memo, useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent as ReactPointerEvent } from "react";
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

/** v3.40 row ladder: 46 standard, 36 compact (DESIGN-SPEC §2.3). */
const ROW_HEIGHT_DEFAULT = 46;
const ROW_HEIGHT_COMPACT = 36;
const CHECKBOX_COLUMN_WIDTH = 22;
const OVERSCAN = 8;
const LOAD_MORE_THRESHOLD = 4;
const LIST_COLUMN_MIN_WIDTHS = [180, 80, 100, 100, 80] as const;
/** Below this container width the decorative type column is dropped. */
const TYPE_COLUMN_MIN_CONTAINER = 900;
/** Below this container width the creation-time column is dropped too. */
const CREATED_COLUMN_MIN_CONTAINER = 700;

function readListColumnWidths() {
  try {
    const parsed = JSON.parse(window.localStorage.getItem("fylura.files.listColumnWidths") ?? "null");
    if (Array.isArray(parsed) && parsed.length === LIST_COLUMN_MIN_WIDTHS.length && parsed.every((value, index) => Number.isFinite(value) && value >= LIST_COLUMN_MIN_WIDTHS[index]! && value <= 1200)) {
      return parsed as number[];
    }
  } catch { /* Use the default list geometry when storage is unavailable. */ }
  return [290, 110, 110, 120, 85];
}

/**
 * Whether every currently loaded row is selected.
 *
 * Counts alone are not enough: a whole-query selection can exclude rows that
 * are still loaded, and a loaded page can be larger than the source's reported
 * selection. The answer comes from the source's membership predicate.
 */
export function loadedSelectionState(interaction: PresentationInteractionProjection) {
  let selected = 0;
  for (let index = 0; index < interaction.loadedRowCount; index += 1) {
    if (interaction.source === "browse") {
      const entry = interaction.entryAt(index);
      if (entry !== undefined && interaction.isSelected(entry)) selected += 1;
    } else {
      const entry = interaction.entryAt(index);
      if (entry !== undefined && interaction.isSelected(entry)) selected += 1;
    }
  }
  const all = interaction.loadedRowCount > 0 && selected === interaction.loadedRowCount;
  return { selected, all, some: selected > 0 && !all };
}

export function listRowHeight(density: string | undefined) {
  return density === "compact" ? ROW_HEIGHT_COMPACT : ROW_HEIGHT_DEFAULT;
}

/** Which optional columns fit the current container. Tracks and cells are
 * dropped together so a hidden column never leaves an empty rail behind. */
export function listColumnSet(showCreatedSource: boolean, containerWidth: number) {
  const showType = containerWidth <= 0 || containerWidth >= TYPE_COLUMN_MIN_CONTAINER;
  const showCreated = showCreatedSource
    && (containerWidth <= 0 || containerWidth >= CREATED_COLUMN_MIN_CONTAINER);
  return { showType, showCreated };
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
  const density = useAppStore((state) => state.density);
  const rowHeight = listRowHeight(density);
  const shellRef = useRef<HTMLDivElement | null>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const rowVirtualizer = useVirtualizer({
    count: interaction.rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: OVERSCAN
  });

  // A density change alters every row's height, so the virtualiser must
  // re-measure instead of keeping the previous estimate.
  useEffect(() => {
    rowVirtualizer.measure();
  }, [rowHeight, rowVirtualizer]);

  useEffect(() => {
    const element = shellRef.current;
    if (!element) return;
    const measure = () => setContainerWidth(element.clientWidth);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
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
    if (event.key === "F5" || ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "r")) {
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
      const nextIndex = nextNavigationIndex(event.key, interaction.focusedIndex, interaction.loadedRowCount, scrollRef.current, rowHeight);
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

  const { showType, showCreated } = listColumnSet(interaction.source === "browse", containerWidth);
  const columns: { field: BrowseSortSpec["field"]; label: string; widthIndex?: number }[] = [
    ...(showType ? [{ field: "type" as const, label: t("fileType"), widthIndex: 1 }] : []),
    ...(showCreated ? [{ field: "created" as const, label: t("u3Created"), widthIndex: 2 }] : []),
    { field: "modified", label: t("fileModified"), widthIndex: 3 },
    { field: "size", label: t("fileSize"), widthIndex: 4 }
  ];
  // The name track flexes between its floor and the stored width so a narrow
  // pane shrinks names before it overflows; the other tracks stay exact.
  const columnsStyle = [
    `${CHECKBOX_COLUMN_WIDTH}px`,
    `minmax(${LIST_COLUMN_MIN_WIDTHS[0]}px, ${columnWidths[0]}px)`,
    ...columns.map((column) => `${columnWidths[column.widthIndex!]}px`)
  ].join(" ");

  // Header selection state asks the source about each loaded entry instead of
  // comparing counts. A whole-query selection can still exclude individual
  // files, so "N >= loaded" would wrongly tick the box for a page that has an
  // unselected row in it.
  const loadedSelection = useMemo(() => loadedSelectionState(interaction), [interaction]);
  const allLoadedSelected = loadedSelection.all;
  const someSelected = loadedSelection.some;
  const selectAllLabel = interaction.capabilities.selectAll === "all_matching"
    ? t("statusSelectAllMatching").replace("{count}", interaction.rowCount.toLocaleString(language))
    : t("statusSelectAllLoaded").replace("{count}", interaction.loadedRowCount.toLocaleString(language));
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
    <div
      ref={shellRef}
      className={cn(
        "shared-file-list-shell",
        showType && "has-type-column",
        showCreated && "has-created-column",
        density === "compact" && "is-compact"
      )}
      data-list-row-height={rowHeight}
      style={{ "--shared-file-list-columns": columnsStyle } as CSSProperties}
    >
      <div className="shared-file-list-header" role="group" aria-label={t("u3Sort")}>
        <div className="shared-file-list-header-cell shared-file-list-select-cell">
          <SelectAllCheckbox
            checked={allLoadedSelected}
            indeterminate={someSelected}
            label={selectAllLabel}
            onChange={(next) => {
              if (next) interaction.actions.selectAll();
              else interaction.actions.clearSelection();
            }}
          />
        </div>
        <div className="shared-file-list-header-cell">
          {onSort ? <button type="button" data-browse-sort-column="name"
            aria-pressed={sort?.field === "name"}
            aria-label={sort?.field === "name" ? `${t("fileName")}, ${sort.direction === "asc" ? t("u3Asc") : t("u3Desc")}` : t("fileName")}
            onClick={() => onSort("name")}>
            {t("fileName")}{sort?.field === "name" ? <span aria-hidden="true">{sort.direction === "asc" ? " ↑" : " ↓"}</span> : null}
          </button> : <span>{t("fileName")}</span>}
        </div>
        {columns.map(({ field, label, widthIndex }, index) => (
          <div key={field} className="shared-file-list-header-cell">
            {onSort ? <button type="button" data-browse-sort-column={field}
              className={field === "size" ? "shared-file-list-size-heading" : undefined}
              aria-pressed={sort?.field === field}
              aria-label={sort?.field === field ? `${label}, ${sort.direction === "asc" ? t("u3Asc") : t("u3Desc")}` : label}
              onClick={() => onSort(field)}>
              {label}{sort?.field === field ? <span aria-hidden="true">{sort.direction === "asc" ? " ↑" : " ↓"}</span> : null}
            </button> : <span className={field === "size" ? "shared-file-list-size-heading" : undefined}>{label}</span>}
            {index < columns.length - 1 ? <span role="separator" aria-orientation="vertical" aria-label={`${label} column width`} aria-valuemin={LIST_COLUMN_MIN_WIDTHS[widthIndex!]} aria-valuemax={1200} aria-valuenow={columnWidths[widthIndex!]} tabIndex={0} className="shared-file-list-column-resize"
              onPointerDown={(event) => resizePointerDown(event, widthIndex!)} onPointerMove={resizePointerMove} onPointerUp={resizePointerUp}
              onPointerCancel={resizePointerUp} onKeyDown={(event) => resizeByKeyboard(event, widthIndex!)} /> : null}
          </div>
        ))}
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
                  showType={showType}
                  showCreated={showCreated}
                  index={virtualRow.index}
                  selected={isInteractionEntrySelected(interaction, entry)}
                  focused={isInteractionEntryFocused(interaction, entry)}
                  language={language}
                  t={t}
                  style={rowStyle(virtualRow.start, virtualRow.size)}
                  onClick={(event) => {
                    scrollRef.current?.focus();
                    const togglesOnly = event.target instanceof Element
                      && event.target.closest("[data-row-select-toggle]") !== null;
                    selectInteractionEntry(
                      interaction,
                      entry,
                      virtualRow.index,
                      togglesOnly ? "toggle" : selectionIntentFromModifiers(event)
                    );
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
  showType,
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
  showType: boolean;
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
      <span className="shared-file-list-select-cell" data-row-select-toggle="true" aria-hidden="true">
        <span className="shared-file-list-checkbox" data-checked={selected ? "true" : "false"}>
          {selected ? <Check size={9} strokeWidth={3} /> : null}
        </span>
      </span>
      <span className="shared-file-list-name">
        <span className="shared-file-list-icon" aria-hidden="true" data-file-kind={fileKindToken(entry, isDirectory)}>
          {isDirectory ? <Folder size={16} /> : <FileText size={16} />}
          {missingLabel ? <AlertTriangle className="shared-file-list-warning-icon" size={13} /> : null}
        </span>
        <span className="shared-file-list-name-copy">
          <strong title={entry.displayName}>{entry.displayName}</strong>
          {details ? <span className={cn(missingLabel && "shared-file-list-warning")} title={details}>{details}</span> : null}
        </span>
      </span>
      {showType ? <span className="shared-file-list-kind">{isDirectory ? kindLabel : entry.typeHint ?? entry.extension ?? kindLabel}</span> : null}
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
  scrollElement: HTMLElement | null,
  rowHeight: number = ROW_HEIGHT_DEFAULT
) {
  if (loadedRowCount === 0) return -1;
  const page = Math.max(1, Math.floor((scrollElement?.clientHeight ?? rowHeight * 8) / rowHeight) - 1);
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

/**
 * The checkbox is a visual affordance inside an ARIA option row. The row keeps
 * the accessible contract (`aria-selected`) and the toggle intent, so screen
 * readers never meet a nested interactive control.
 */
function SelectAllCheckbox({
  checked,
  indeterminate,
  label,
  onChange
}: {
  checked: boolean;
  indeterminate: boolean;
  label: string;
  onChange: (next: boolean) => void;
}) {
  const ref = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = indeterminate;
  }, [indeterminate]);
  return (
    <input
      ref={ref}
      type="checkbox"
      className="shared-file-list-select-all"
      checked={checked}
      aria-label={label}
      title={label}
      onChange={(event) => onChange(event.currentTarget.checked)}
    />
  );
}

const FILE_KIND_BY_EXTENSION: Record<string, string> = {
  pdf: "pdf",
  doc: "doc", docx: "doc", rtf: "doc", txt: "doc", md: "doc", key: "deck",
  xls: "sheet", xlsx: "sheet", csv: "sheet", numbers: "sheet",
  ppt: "deck", pptx: "deck",
  png: "img", jpg: "img", jpeg: "img", gif: "img", webp: "img", heic: "img", svg: "img",
  mp4: "video", mov: "video", mkv: "video", avi: "video", webm: "video",
  mp3: "audio", wav: "audio", flac: "audio", aac: "audio", m4a: "audio",
  js: "code", ts: "code", tsx: "code", jsx: "code", json: "code", py: "code", rs: "code", css: "code", html: "code",
  zip: "zip", gz: "zip", tar: "zip", "7z": "zip", rar: "zip", dmg: "zip"
};

export function fileKindToken(entry: PresentationEntry, isDirectory: boolean) {
  if (isDirectory) return "folder";
  const extension = (entry.extension ?? "").toLocaleLowerCase();
  return FILE_KIND_BY_EXTENSION[extension] ?? "file";
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
