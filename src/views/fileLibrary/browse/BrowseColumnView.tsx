import { Folder } from "lucide-react";
import type { Translator } from "../../../types/ui";
import type { BrowsePresentationEntry } from "../presentation/contracts";
import type { PresentationInteractionProjection } from "../list/interactionContracts";
import type { BrowseSourceOwner } from "./browseSourceOwner";
import { cn } from "../../../utils/tw";
import "./browseColumnView.css";

/**
 * Finder-style column browsing inside a single pane.
 *
 * Columns come from the pane's own navigation stack (`breadcrumbs`), never from
 * a second enumeration session: an ancestor column shows the entries the pane
 * actually visited, and the last column is the live listing. When an ancestor
 * has no retained snapshot the column says so instead of inventing contents.
 *
 * This is a presentation of the existing Browse authority. Selection still goes
 * through the pane's interaction projection and navigation still goes through
 * the workspace session, so a pane keeps its own path, history and selection.
 */
export function BrowseColumnView({
  source,
  interaction,
  t,
  cachedColumns,
  onActivate,
  onNavigateCrumb
}: {
  source: BrowseSourceOwner;
  interaction: PresentationInteractionProjection;
  t: Translator;
  cachedColumns: ReadonlyMap<string, readonly BrowsePresentationEntry[]>;
  onActivate: (entry: BrowsePresentationEntry) => void;
  onNavigateCrumb: (index: number) => void;
}) {
  const columns = source.breadcrumbs.map((crumb, index) => {
    const isCurrent = index === source.breadcrumbs.length - 1;
    const entries = isCurrent
      ? (source.entries as readonly BrowsePresentationEntry[])
      : cachedColumns.get(crumb.pathRef.id) ?? null;
    return { crumb, index, isCurrent, entries };
  });

  return (
    <div
      className="browse-columns"
      data-browse-columns="true"
      data-browse-column-count={columns.length}
      aria-label={t("browseColumnViewLabel")}
    >
      {columns.map(({ crumb, index, isCurrent, entries }) => (
        <section
          className="browse-column"
          key={`${crumb.sessionId}:${crumb.pathRef.id}`}
          aria-label={crumb.label}
          data-browse-column-current={isCurrent ? "true" : undefined}
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
          </header>
          {entries === null ? (
            <div className="browse-column-empty">
              <button type="button" onClick={() => onNavigateCrumb(index)}>{t("browseColumnLoadHint")}</button>
            </div>
          ) : entries.length === 0 ? (
            <div className="browse-column-empty">{t("browseColumnEmpty")}</div>
          ) : (
            <ul className="browse-column-list" role="listbox" aria-label={crumb.label} aria-multiselectable="true">
              {entries.map((entry, entryIndex) => {
                const selected = isCurrent
                  && entry.source === "browse"
                  && source.selectedIds.has(entry.entryRef.entryId);
                const folder = entry.entryKind === "directory";
                return (
                  <li key={entry.renderKey}>
                    <button
                      type="button"
                      className={cn("browse-column-row", selected && "is-selected")}
                      role="option"
                      aria-selected={selected}
                      aria-label={`${entry.displayName}, ${folder ? t("browseFolder") : t("browseFile")}`}
                      data-browse-column-entry={entry.source === "browse" ? entry.entryRef.entryId : undefined}
                      onClick={() => {
                        if (folder) {
                          onActivate(entry);
                          return;
                        }
                        if (interaction.source === "browse") {
                          interaction.actions.select(entry, entryIndex, "replace");
                        }
                      }}
                      onDoubleClick={() => onActivate(entry)}
                    >
                      {folder ? <Folder size={14} aria-hidden="true" className="browse-column-icon" /> : null}
                      <span className="browse-column-name" title={entry.displayName}>{entry.displayName}</span>
                      {folder ? <span className="browse-column-chevron" aria-hidden="true">›</span> : null}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      ))}
      <span className="sr-only" aria-live="polite">
        {source.breadcrumbs.at(-1)?.label ?? ""}
      </span>
    </div>
  );
}
