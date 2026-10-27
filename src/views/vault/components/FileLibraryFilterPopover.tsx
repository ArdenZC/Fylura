import { usePane, usePaneQuerySelector } from "../../fileLibrary/workbench/PaneContext";
import { useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { FileQueryFiltersV2, FileType, Lifecycle, RiskLevel, UserTag } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { selectSurface, buttonSecondary, buttonGhost, cn } from "../../../utils/tw";

const FILTER_POPOVER_GUTTER = 12;
const FILTER_POPOVER_ANCHOR_GAP = 8;
const FILTER_POPOVER_MAX_WIDTH = 380;
const FILTER_POPOVER_MAX_HEIGHT = 560;
const FILTER_POPOVER_FLIP_THRESHOLD = 320;
const FILTER_POPOVER_TRIGGER_SELECTOR = '[aria-controls^="library-filter-popover"][aria-expanded="true"]';
const FILTER_POPOVER_FOCUSABLE_SELECTOR = "button:not([disabled]), select:not([disabled]), input:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex='-1'])";

type RectLike = Pick<DOMRect, "left" | "right" | "top" | "bottom">;

type FilterPopoverPlacement = {
  left: number;
  width: number;
  maxHeight: number;
  side: "above" | "below";
  top?: number;
  bottom?: number;
};

export function computeFileLibraryFilterPopoverPlacement({
  anchor,
  boundary,
  viewportWidth,
  viewportHeight
}: {
  anchor: RectLike;
  boundary: RectLike;
  viewportWidth: number;
  viewportHeight: number;
}): FilterPopoverPlacement {
  const safeLeft = Math.max(FILTER_POPOVER_GUTTER, boundary.left + FILTER_POPOVER_GUTTER);
  const safeRight = Math.min(viewportWidth - FILTER_POPOVER_GUTTER, boundary.right - FILTER_POPOVER_GUTTER);
  const safeTop = Math.max(FILTER_POPOVER_GUTTER, boundary.top + FILTER_POPOVER_GUTTER);
  const safeBottom = Math.min(viewportHeight - FILTER_POPOVER_GUTTER, boundary.bottom - FILTER_POPOVER_GUTTER);
  const availableWidth = Math.max(0, safeRight - safeLeft);
  const width = Math.min(FILTER_POPOVER_MAX_WIDTH, availableWidth);
  const maxLeft = Math.max(safeLeft, safeRight - width);
  const left = Math.min(Math.max(anchor.right - width, safeLeft), maxLeft);

  const belowTop = Math.min(safeBottom, anchor.bottom + FILTER_POPOVER_ANCHOR_GAP);
  const aboveBottom = Math.max(safeTop, anchor.top - FILTER_POPOVER_ANCHOR_GAP);
  const belowSpace = Math.max(0, safeBottom - belowTop);
  const aboveSpace = Math.max(0, aboveBottom - safeTop);
  const side = belowSpace >= FILTER_POPOVER_FLIP_THRESHOLD || belowSpace >= aboveSpace ? "below" : "above";
  const maxHeight = Math.min(FILTER_POPOVER_MAX_HEIGHT, side === "below" ? belowSpace : aboveSpace);

  return side === "below"
    ? { left, width, maxHeight, side, top: belowTop }
    : { left, width, maxHeight, side, bottom: Math.max(0, viewportHeight - aboveBottom) };
}

export function FileLibraryFilterPopover({
  filters,
  tags,
  t,
  onFiltersChange,
  onClear,
  onClose
}: {
  filters: FileQueryFiltersV2;
  tags: UserTag[];
  t: Translator;
  onFiltersChange: (value: Partial<FileQueryFiltersV2>) => void;
  onClear: () => void;
  onClose: () => void;
}) {
  const query = usePaneQuerySelector();
  const pane = usePane();
  const titleId = pane ? `library-filter-title-${pane.id}` : "library-filter-title";
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [placement, setPlacement] = useState<FilterPopoverPlacement | null>(null);

  useLayoutEffect(() => {
    const trigger = query(FILTER_POPOVER_TRIGGER_SELECTOR);
    if (!trigger) return undefined;
    const boundary = trigger.closest<HTMLElement>(".file-library-workspace");

    const updatePlacement = () => {
      const nextTrigger = query(FILTER_POPOVER_TRIGGER_SELECTOR);
      if (!nextTrigger) return;
      const nextBoundary = nextTrigger.closest<HTMLElement>(".file-library-workspace") ?? boundary;
      const boundaryRect = nextBoundary?.getBoundingClientRect() ?? {
        left: 0,
        right: window.innerWidth,
        top: 0,
        bottom: window.innerHeight
      };
      setPlacement(computeFileLibraryFilterPopoverPlacement({
        anchor: nextTrigger.getBoundingClientRect(),
        boundary: boundaryRect,
        viewportWidth: window.innerWidth,
        viewportHeight: window.innerHeight
      }));
    };

    updatePlacement();
    window.addEventListener("resize", updatePlacement);
    window.addEventListener("scroll", updatePlacement, true);
    window.visualViewport?.addEventListener("resize", updatePlacement);
    window.visualViewport?.addEventListener("scroll", updatePlacement);

    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updatePlacement);
    observer?.observe(trigger);
    if (boundary) observer?.observe(boundary);

    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", updatePlacement);
      window.removeEventListener("scroll", updatePlacement, true);
      window.visualViewport?.removeEventListener("resize", updatePlacement);
      window.visualViewport?.removeEventListener("scroll", updatePlacement);
    };
  }, [query]);

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== "Tab") return;

    const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>(FILTER_POPOVER_FOCUSABLE_SELECTOR)]
      .filter((element) => element.getClientRects().length > 0 && element.getAttribute("aria-hidden") !== "true");
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  if (!placement || placement.width <= 0 || placement.maxHeight <= 0) return null;

  return (
    <div
      ref={panelRef}
      className="z-[60] flex flex-col overflow-y-auto overscroll-contain rounded-[var(--zc-radius-floating)] border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] p-4 text-[var(--zc-text-primary)] shadow-[var(--zc-shadow-menu)]"
      style={{
        position: "fixed",
        left: placement.left,
        width: placement.width,
        maxHeight: placement.maxHeight,
        ...(placement.side === "below" ? { top: placement.top } : { bottom: placement.bottom })
      }}
      role="dialog"
      aria-labelledby={titleId}
      data-filter-popover-placement={placement.side}
      onKeyDown={handleKeyDown}
    >
      <div className="flex items-start justify-between gap-3"><div><h2 id={titleId} className="text-sm font-semibold">{t("libraryFilterTitle")}</h2><p className="mt-1 text-xs text-[var(--zc-text-secondary)]">{t("libraryScopeHint")}</p></div><button type="button" className={buttonGhost} onClick={onClear}>{t("libraryFilterClear")}</button></div>
      <div className="mt-4 grid gap-3">
        <FilterSelect autoFocus label={t("libraryFilterFileType")} value={filters.fileTypes[0] ?? "all"} onChange={(value) => onFiltersChange({ fileTypes: value === "all" ? [] : [value as FileType] })} options={fileTypeOptions(t)} />
        <FilterSelect label={t("libraryFilterLifecycle")} value={filters.lifecycles[0] ?? "all"} onChange={(value) => onFiltersChange({ lifecycles: value === "all" ? [] : [value as Lifecycle] })} options={lifecycleOptions(t)} />
        <FilterSelect label={t("libraryFilterRisk")} value={filters.risks[0] ?? "all"} onChange={(value) => onFiltersChange({ risks: value === "all" ? [] : [value as RiskLevel] })} options={riskOptions(t)} />
        <FilterSelect label={t("libraryFilterDuplicate")} value={filters.duplicate} onChange={(value) => onFiltersChange({ duplicate: value as FileQueryFiltersV2["duplicate"] })} options={matchModeOptions(t, "duplicate")} />
        <FilterSelect label={t("libraryFilterReview")} value={filters.review} onChange={(value) => onFiltersChange({ review: value as FileQueryFiltersV2["review"] })} options={matchModeOptions(t, "review")} />
        <TagSelect label={t("libraryFilterTagsAll")} value={filters.tagsAllOf} tags={tags} onChange={(value) => onFiltersChange({ tagsAllOf: value })} />
        <TagSelect label={t("libraryFilterTagsAny")} value={filters.tagsAnyOf} tags={tags} onChange={(value) => onFiltersChange({ tagsAnyOf: value })} />
        <TagSelect label={t("libraryFilterTagsExclude")} value={filters.tagsNoneOf} tags={tags} onChange={(value) => onFiltersChange({ tagsNoneOf: value })} />
      </div>
      <div className="sticky bottom-0 mt-4 flex justify-end bg-[var(--zc-surface-overlay)] pt-2"><button type="button" className={cn(buttonSecondary, "min-h-9 px-3 py-1.5 text-xs")} onClick={onClose}>{t("libraryFilterDone")}</button></div>
    </div>
  );
}

function FilterSelect({ label, value, options, onChange, autoFocus = false }: { label: string; value: string; options: Array<readonly [string, string]>; onChange: (value: string) => void; autoFocus?: boolean }) {
  return <label className="grid gap-1 text-xs font-semibold text-[var(--zc-text-tertiary)]"><span>{label}</span><select autoFocus={autoFocus} className={cn(selectSurface, "min-h-9 py-1.5 text-sm")} value={value} onChange={(event) => onChange(event.target.value)}>{options.map(([option, optionLabel]) => <option value={option} key={option}>{optionLabel}</option>)}</select></label>;
}

function TagSelect({ label, value, tags, onChange }: { label: string; value: string[]; tags: UserTag[]; onChange: (value: string[]) => void }) {
  return <label className="grid gap-1 text-xs font-semibold text-[var(--zc-text-tertiary)]"><span>{label}</span><select multiple className={cn(selectSurface, "min-h-20 py-1.5 text-sm")} value={value} onChange={(event) => onChange([...event.currentTarget.selectedOptions].map((option) => option.value))}>{tags.map((tag) => <option key={tag.id} value={tag.id}>{tag.displayName}</option>)}</select></label>;
}

function fileTypeOptions(t: Translator): Array<readonly [string, string]> {
  const values: FileType[] = ["Document", "Image", "Video", "Audio", "Code", "ArchivePackage", "Installer", "Spreadsheet", "Presentation", "Other"];
  return [["all", t("libraryFilterAllOptions")], ...values.map((value) => [`${value}`, t(`libraryType${value === "ArchivePackage" ? "Archive" : value}` as Parameters<Translator>[0])] as const)];
}

function lifecycleOptions(t: Translator): Array<readonly [string, string]> {
  const values: Lifecycle[] = ["Inbox", "Active", "Reference", "Archive", "Disposable", "Duplicate", "Sensitive", "TrashReview", "Unknown"];
  return [["all", t("libraryFilterAllOptions")], ...values.map((value) => [`${value}`, t(`libraryLifecycle${value}` as Parameters<Translator>[0])] as const)];
}

function riskOptions(t: Translator): Array<readonly [string, string]> {
  const values: RiskLevel[] = ["Normal", "Sensitive", "System", "Caution", "Unknown"];
  return [["all", t("libraryFilterAllOptions")], ...values.map((value) => [`${value}`, t(`libraryRisk${value}` as Parameters<Translator>[0])] as const)];
}

function matchModeOptions(t: Translator, kind: "duplicate" | "review"): Array<readonly [string, string]> {
  return [["any", t(kind === "duplicate" ? "libraryFilterDuplicate" : "libraryFilterReview")], ["only", kind === "duplicate" ? t("libraryFilterDuplicateOnly") : t("libraryFilterReviewOnly")], ["exclude", `${t(kind === "duplicate" ? "libraryFilterDuplicate" : "libraryFilterReview")} · ${t("libraryFilterExclude")}`]];
}
