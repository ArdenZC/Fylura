import type {
  FileLibrarySelectionSummary,
  LibrarySelectionV1
} from "../../../types/domain";
import type { Language } from "../../../i18n";
import type { Translator } from "../../../types/ui";
import type { BrowsePresentationEntry } from "../presentation/contracts";
import type { FileLibraryInspectorProps } from "../../vault/components/FileLibraryInspector";

export type ContextPanelContentKind = "none" | "inspector" | "selection-summary";

export interface LibraryContextProjection {
  readonly source: "library";
  readonly kind: ContextPanelContentKind;
  readonly selection: LibrarySelectionV1 | null;
  /** Existing Inspector authority projected through the shared panel seam. */
  readonly inspector: FileLibraryInspectorProps;
}

export interface BrowseContextSizeProjection {
  readonly state: "exact" | "partial" | "unknown";
  readonly total: number | null;
}

export interface BrowseContextTypeCount {
  readonly label: string;
  readonly count: number;
}

export interface BrowseContextProjection {
  readonly source: "browse";
  readonly onPreview?: (entry: BrowsePresentationEntry, trigger: HTMLElement) => void;
  readonly kind: ContextPanelContentKind;
  readonly selectedEntries: readonly BrowsePresentationEntry[];
  readonly selectedCount: number;
  readonly locationLabel: string;
  readonly size: BrowseContextSizeProjection;
  readonly typeCounts: readonly BrowseContextTypeCount[];
  readonly language: Language;
  readonly t: Translator;
}

export type ContextPanelProjection = LibraryContextProjection | BrowseContextProjection;

export function libraryContextContentKind(selection: LibrarySelectionV1 | null): ContextPanelContentKind {
  if (selection === null) return "none";
  if (selection.kind === "all_matching") return "selection-summary";
  return selection.fileIds.length === 1 ? "inspector" : "selection-summary";
}

export function libraryContextSelectionCount(
  selection: LibrarySelectionV1 | null,
  summary: FileLibrarySelectionSummary | null
) {
  if (selection === null) return null;
  if (selection.kind === "all_matching") return summary?.count ?? null;
  return selection.fileIds.length;
}

export function createLibraryContextProjection(
  selection: LibrarySelectionV1 | null,
  inspector: FileLibraryInspectorProps
): LibraryContextProjection {
  return {
    source: "library",
    kind: libraryContextContentKind(selection),
    selection,
    inspector: {
      ...inspector,
      selectionKind: selection?.kind ?? null,
      selectedCount: libraryContextSelectionCount(selection, inspector.selectionSummary)
    }
  };
}

export function createBrowseContextProjection({
  entries,
  selectedIds,
  locationLabel,
  language,
  t,
  onPreview
}: {
  onPreview?: (entry: BrowsePresentationEntry, trigger: HTMLElement) => void;
  entries: readonly BrowsePresentationEntry[];
  selectedIds: ReadonlySet<string>;
  locationLabel: string;
  language: Language;
  t: Translator;
}): BrowseContextProjection {
  const selectedEntries = entries.filter((entry) => selectedIds.has(entry.entryRef.entryId));
  const selectedCount = selectedEntries.length;
  const size = browseSizeProjection(selectedEntries);
  const typeCounts = browseTypeCounts(selectedEntries);
  return {
    source: "browse",
    onPreview,
    kind: selectedCount === 0 ? "none" : selectedCount === 1 ? "inspector" : "selection-summary",
    selectedEntries,
    selectedCount,
    locationLabel,
    size,
    typeCounts,
    language,
    t
  };
}

export function browseSizeProjection(entries: readonly BrowsePresentationEntry[]): BrowseContextSizeProjection {
  if (entries.length === 0) return { state: "unknown", total: null };
  const knownSizes = entries.filter((entry) => typeof entry.size === "number" && Number.isFinite(entry.size));
  if (knownSizes.length === 0) return { state: "unknown", total: null };
  const total = knownSizes.reduce((sum, entry) => sum + (entry.size ?? 0), 0);
  return {
    state: knownSizes.length === entries.length ? "exact" : "partial",
    total
  };
}

export function browseTypeCounts(entries: readonly BrowsePresentationEntry[]): BrowseContextTypeCount[] {
  const counts = new Map<string, number>();
  for (const entry of entries) {
    const label = entry.typeHint ?? entry.extension ?? entry.entryKind;
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([label, count]) => ({ label, count }));
}

export function browsePresentationEntryLabel(entry: BrowsePresentationEntry) {
  return entry.typeHint ?? entry.extension ?? entry.entryKind;
}

export function browseSelectedSummaryText(
  projection: Pick<BrowseContextProjection, "selectedCount" | "size" | "t">
) {
  const sizeLabel = projection.size.state === "exact"
    ? projection.t("fileLibraryContextKnownSize")
    : projection.size.state === "partial"
      ? projection.t("fileLibraryContextPartialSize")
      : projection.t("fileLibraryContextUnknownSize");
  return `${projection.t("browseSelectionLoaded").replace("{count}", String(projection.selectedCount))} · ${sizeLabel}`;
}
