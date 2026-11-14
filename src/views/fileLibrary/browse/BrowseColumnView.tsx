import { useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { Folder, LoaderCircle } from "lucide-react";
import type { Translator } from "../../../types/ui";
import type { BrowsePresentationEntry } from "../presentation/contracts";
import type { BrowseInteractionProjection } from "../list/interactionContracts";
import type { BrowseBreadcrumb, BrowseSourceOwner } from "./browseSourceOwner";
import { cn } from "../../../utils/tw";
import "./browseColumnView.css";

/** Rows rendered per column before an explicit "show more" step. Columns stay
 * bounded even when the owning enumeration has loaded thousands of entries. */
export const COLUMN_ROW_WINDOW = 300;

/**
 * Finder-style column browsing inside a single pane.
 *
 * Columns come from the pane's own navigation stack (`breadcrumbs`), never from
 * a second enumeration session: an ancestor column shows the entries the pane
 * actually visited, and the last column is the live listing. When an ancestor
 * has no retained snapshot the column says so instead of inventing contents.
 *
 * The current column owns every interactive affordance of the other views:
 * selection, open, Space Quick Preview, arrow-key focus, context menu and the
 * owning pagination action. Ancestor columns are navigation: a folder opens
 * from its own ancestor chain, while a file is inert because acting on it
 * through the current selection would target a different directory.
 */
export function BrowseColumnView({
  source,
  interaction,
  t,
  cachedColumns,
  showHidden,
  onActivate,
  onNavigateCrumb,
  onPreview,
  onContextMenu,
  onOpenContextMenu,
  onEscape
}: {
  source: BrowseSourceOwner;
  interaction: BrowseInteractionProjection;
  t: Translator;
  cachedColumns: ReadonlyMap<string, readonly BrowsePresentationEntry[]>;
  showHidden: boolean;
  /** `ancestor` is the column the entry was activated from, or null when the
   * activation came from the current directory. */
  onActivate: (entry: BrowsePresentationEntry, ancestor: BrowseBreadcrumb | null) => void;
  onNavigateCrumb: (index: number) => void;
  onPreview: (entry: BrowsePresentationEntry, trigger: HTMLElement, event: KeyboardEvent<HTMLElement>) => boolean | void;
  onContextMenu: (event: MouseEvent<HTMLElement>, entry: BrowsePresentationEntry) => void;
  onOpenContextMenu: (entry: BrowsePresentationEntry, index: number) => void;
  onEscape?: () => boolean;
}) {
  const columns = source.breadcrumbs.map((crumb, index) => {
    const isCurrent = index === source.breadcrumbs.length - 1;
    const cached = isCurrent ? source.entries : cachedColumns.get(crumb.pathRef.id) ?? null;
    const entries = cached === null
      ? null
      : showHidden ? cached : cached.filter((entry) => !entry.isHidden);
    return { crumb, index, isCurrent, entries, rawCount: cached?.length ?? 0 };
  });

  return (
    <div
      className="browse-columns"
      data-browse-columns="true"
      data-browse-column-count={columns.length}
      aria-label={t("browseColumnViewLabel")}
    >
      {columns.map(({ crumb, index, isCurrent, entries, rawCount }) => (
        <BrowseColumn
          key={`${crumb.sessionId}:${crumb.pathRef.id}`}
          crumb={crumb}
          index={index}
          isCurrent={isCurrent}
          entries={entries}
          rawCount={rawCount}
          selectedIds={source.selectedIds}
          focusedId={source.focusedId}
          interaction={interaction}
          loadedCount={source.loadedCount}
          knownCount={source.knownCount}
          t={t}
          onActivate={onActivate}
          onNavigateCrumb={onNavigateCrumb}
          onPreview={onPreview}
          onContextMenu={onContextMenu}
          onOpenContextMenu={onOpenContextMenu}
          onEscape={onEscape}
        />
      ))}
      <span className="sr-only" aria-live="polite">
        {source.breadcrumbs.at(-1)?.label ?? ""}
      </span>
    </div>
  );
}

function BrowseColumn({
  crumb,
  index,
  isCurrent,
  entries,
  rawCount,
  selectedIds,
  focusedId,
  interaction,
  loadedCount,
  knownCount,
  t,
  onActivate,
  onNavigateCrumb,
  onPreview,
  onContextMenu,
  onOpenContextMenu,
  onEscape
}: {
  crumb: BrowseBreadcrumb;
  index: number;
  isCurrent: boolean;
  entries: readonly BrowsePresentationEntry[] | null;
  /** Entries before the hidden-file filter, used to explain an empty column. */
  rawCount: number;
  selectedIds: ReadonlySet<string>;
  focusedId: string | null;
  interaction: BrowseInteractionProjection;
  loadedCount: number;
  knownCount: number | null;
  t: Translator;
  onActivate: (entry: BrowsePresentationEntry, ancestor: BrowseBreadcrumb | null) => void;
  onNavigateCrumb: (index: number) => void;
  onPreview: (entry: BrowsePresentationEntry, trigger: HTMLElement, event: KeyboardEvent<HTMLElement>) => boolean | void;
  onContextMenu: (event: MouseEvent<HTMLElement>, entry: BrowsePresentationEntry) => void;
  onOpenContextMenu: (entry: BrowsePresentationEntry, index: number) => void;
  onEscape?: () => boolean;
}) {
  const [limit, setLimit] = useState(COLUMN_ROW_WINDOW);
  const listRef = useRef<HTMLUListElement | null>(null);
  const visible = useMemo(() => entries === null ? null : entries.slice(0, limit), [entries, limit]);

  const countTitle = knownCount === null
    ? t("statusLoadScopeLoaded").replace("{loaded}", loadedCount.toLocaleString())
    : t("statusLoadedOfTotal")
      .replace("{loaded}", loadedCount.toLocaleString())
      .replace("{total}", knownCount.toLocaleString());

  function focusRow(nextIndex: number) {
    if (visible === null) return;
    const entry = visible[nextIndex];
    if (entry === undefined) return;
    interaction.actions.focus(entry, nextIndex);
    const row = listRef.current?.querySelector<HTMLElement>(`[data-column-row-index="${nextIndex}"]`);
    row?.focus();
    row?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }

  function handleKeyDown(event: KeyboardEvent<HTMLUListElement>) {
    if (!isCurrent) return;
    const row = event.target instanceof Element
      ? event.target.closest<HTMLElement>("[data-column-row-index]")
      : null;
    const currentIndex = row === null
      ? interaction.focusedIndex
      : Number(row.dataset.columnRowIndex);
    const total = visible?.length ?? 0;

    if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Home" || event.key === "End") {
      if (total === 0) return;
      event.preventDefault();
      const next = event.key === "Home"
        ? 0
        : event.key === "End"
          ? total - 1
          : Math.min(total - 1, Math.max(0, (currentIndex < 0 ? 0 : currentIndex) + (event.key === "ArrowDown" ? 1 : -1)));
      focusRow(next);
      return;
    }
    if (event.key === "Escape" && onEscape?.()) {
      event.preventDefault();
      return;
    }
    const entry = currentIndex >= 0 ? visible?.[currentIndex] : undefined;
    if (entry === undefined) return;
    if (event.key === "Enter") {
      event.preventDefault();
      onActivate(entry, null);
      return;
    }
    if (event.key === " " || event.key === "Space") {
      // The preview controller refuses an event whose default is already
      // prevented, so the order matters: offer it first, consume only if taken.
      const handled = onPreview(entry, listRef.current ?? document.body, event) === true;
      if (handled) event.preventDefault();
      return;
    }
    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
      event.preventDefault();
      interaction.actions.focus(entry, currentIndex);
      interaction.actions.select(entry, currentIndex, "replace");
      onOpenContextMenu(entry, currentIndex);
    }
  }

  return (
    <section
      className="browse-column"
      aria-label={crumb.label}
      data-browse-column-current={isCurrent ? "true" : undefined}
      data-browse-column-ancestor={isCurrent ? undefined : "true"}
    >
      <header className="browse-column-head">
        <button
          type="button"
          className="browse-column-title"
          title={crumb.label}
          onClick={() => onNavigateCrumb(index)}
        >
          {crumb.label}
        </button>
        {isCurrent ? (
          <span className="browse-column-count" title={countTitle} data-browse-column-count-label="true">
            {knownCount === null
              ? loadedCount.toLocaleString()
              : t("browseColumnCountCompact")
                .replace("{loaded}", loadedCount.toLocaleString())
                .replace("{total}", knownCount.toLocaleString())}
          </span>
        ) : null}
      </header>
      {entries === null ? (
        <div className="browse-column-empty">
          <button type="button" onClick={() => onNavigateCrumb(index)}>{t("browseColumnLoadHint")}</button>
        </div>
      ) : entries.length === 0 ? (
        <div className="browse-column-empty">
          {rawCount > 0 ? t("browseColumnHiddenEmpty") : t("browseColumnEmpty")}
        </div>
      ) : (
        <>
          <ul
            ref={listRef}
            className="browse-column-list"
            role={isCurrent ? "listbox" : undefined}
            aria-label={crumb.label}
            aria-multiselectable={isCurrent ? "true" : undefined}
            onKeyDown={isCurrent ? handleKeyDown : undefined}
          >
            {(visible ?? []).map((entry, entryIndex) => {
              const selected = isCurrent
                && entry.source === "browse"
                && selectedIds.has(entry.entryRef.entryId);
              const focused = isCurrent && entry.entryRef.entryId === focusedId;
              const folder = entry.entryKind === "directory";
              const shared = {
                className: cn("browse-column-row", selected && "is-selected", focused && "is-focused"),
                title: entry.displayName,
                "data-column-row-index": entryIndex,
                "data-browse-entry-id": isCurrent && entry.source === "browse" ? entry.entryRef.entryId : undefined,
                onDoubleClick: () => onActivate(entry, isCurrent ? null : crumb)
              };

              if (!isCurrent && !folder) {
                // Acting on a file requires the directory that owns the pane's
                // selection; an ancestor row stays inert rather than silently
                // targeting the current folder.
                return (
                  <li key={entry.renderKey}>
                    <span
                      {...shared}
                      className={cn(shared.className, "is-static")}
                      title={t("browseColumnAncestorFileHint")}
                      onDoubleClick={undefined}
                    >
                      <span className="browse-column-name" title={entry.displayName}>{entry.displayName}</span>
                    </span>
                  </li>
                );
              }

              return (
                <li key={entry.renderKey}>
                  <button
                    type="button"
                    {...shared}
                    role={isCurrent ? "option" : undefined}
                    aria-selected={isCurrent ? selected : undefined}
                    aria-label={`${entry.displayName}, ${folder ? t("browseFolder") : t("browseFile")}`}
                    onClick={() => {
                      if (folder) {
                        onActivate(entry, isCurrent ? null : crumb);
                        return;
                      }
                      if (!isCurrent) return;
                      interaction.actions.focus(entry, entryIndex);
                      interaction.actions.select(entry, entryIndex, "replace");
                    }}
                    onContextMenu={(event) => {
                      if (isCurrent) onContextMenu(event, entry);
                    }}
                  >
                    {folder ? <Folder size={14} aria-hidden="true" className="browse-column-icon" /> : null}
                    <span className="browse-column-name" title={entry.displayName}>{entry.displayName}</span>
                    {folder ? <span className="browse-column-chevron" aria-hidden="true">›</span> : null}
                  </button>
                </li>
              );
            })}
          </ul>
          {visible !== null && entries.length > visible.length ? (
            <button
              type="button"
              className="browse-column-show-more"
              onClick={() => setLimit((current) => current + COLUMN_ROW_WINDOW)}
            >
              {t("browseColumnShowMore").replace("{count}", String(Math.min(COLUMN_ROW_WINDOW, entries.length - visible.length)))}
            </button>
          ) : null}
          {isCurrent && interaction.hasMore ? (
            <button
              type="button"
              className="browse-column-load-more"
              disabled={interaction.isLoadingMore}
              onClick={() => void interaction.actions.loadMore()}
            >
              {interaction.isLoadingMore
                ? <><LoaderCircle size={12} className="animate-spin" aria-hidden="true" />{t("browseEnumerationLoadingMore")}</>
                : t("browseLoadMore")}
            </button>
          ) : null}
        </>
      )}
    </section>
  );
}
