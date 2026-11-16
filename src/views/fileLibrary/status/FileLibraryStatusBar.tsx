import type { ReactNode } from "react";
import { cn } from "../../../utils/tw";
import { formatBytes } from "../../../utils/format";
import type { Translator } from "../../../types/ui";
import "./fileLibraryStatusBar.css";

export type StatusSelectionScope = "none" | "loaded" | "all";

/**
 * The Files status bar. Every number is derived from the source that owns it:
 *
 * - `totalCount` is the source's exact logical count when it has published one.
 *   A loaded page is never promoted to a directory total.
 * - `selectionCount` is the source's selection, which may be larger than the
 *   mounted rows; `selectionScope` names that difference instead of hiding it.
 *   A whole-query selection whose total is not published yet passes `null`
 *   rather than 0, so "counting" is never rendered as "nothing selected".
 * - `selectedSize` is only rendered when every selected entry was loaded and
 *   reported a size, so a partial sum is never shown as the selection size.
 */
export function FileLibraryStatusBar({
  loadedCount,
  totalCount,
  selectionCount,
  selectionScope,
  selectedSize,
  statusText,
  sourceText,
  actions,
  live = false,
  t,
  className
}: {
  loadedCount: number;
  totalCount: number | null;
  selectionCount: number | null;
  selectionScope: StatusSelectionScope;
  selectedSize: number | null;
  statusText?: string | null;
  sourceText?: string | null;
  actions?: ReactNode;
  live?: boolean;
  t: Translator;
  className?: string;
}) {
  const countText = statusCountText({ loadedCount, totalCount, t });
  const selectionText = statusSelectionText({ selectionCount, selectionScope, selectedSize, t });

  return (
    <footer className={cn("fl-status", className)} data-file-library-status="true">
      <span className="fl-status-count" title={countText}>{countText}</span>
      <span className="fl-status-selection" data-selection-scope={selectionScope}>{selectionText}</span>
      {statusText ? (
        <span
          className="fl-status-text"
          role={live ? "status" : undefined}
          aria-live={live ? "polite" : undefined}
          data-browse-enumeration-status={live ? "true" : undefined}
        >
          {statusText}
        </span>
      ) : null}
      <span className="fl-status-spacer" aria-hidden="true" />
      {sourceText ? <span className="fl-status-source" title={sourceText}>{sourceText}</span> : null}
      {actions ? <span className="fl-status-actions">{actions}</span> : null}
    </footer>
  );
}

export function statusCountText({
  loadedCount,
  totalCount,
  t
}: {
  loadedCount: number;
  totalCount: number | null;
  t: Translator;
}) {
  if (totalCount === null) return t("statusItems").replace("{count}", loadedCount.toLocaleString());
  if (totalCount === loadedCount) return t("statusItems").replace("{count}", totalCount.toLocaleString());
  return t("statusLoadedOfTotal")
    .replace("{loaded}", loadedCount.toLocaleString())
    .replace("{total}", totalCount.toLocaleString());
}

export function statusSelectionText({
  selectionCount,
  selectionScope,
  selectedSize,
  t
}: {
  selectionCount: number | null;
  selectionScope: StatusSelectionScope;
  selectedSize: number | null;
  t: Translator;
}) {
  if (selectionScope === "none") return t("statusNoSelection");
  // The whole-query selection is established but its exact count is still
  // pending: say so instead of reporting zero selected items.
  if (selectionCount === null) return t("statusSelectedAllPending");
  if (selectionCount <= 0) return t("statusNoSelection");
  const base = selectionScope === "all"
    ? t("statusSelectedAll").replace("{count}", selectionCount.toLocaleString())
    : t("statusSelected").replace("{count}", selectionCount.toLocaleString());
  return selectedSize === null ? base : `${base} · ${formatBytes(selectedSize)}`;
}
