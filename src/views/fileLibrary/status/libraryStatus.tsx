import type { Translator } from "../../../types/ui";
import { replaceCopy } from "../library/libraryModeCopy";
import type { LibrarySourceOwner } from "../library/librarySourceOwner";
import { FileLibraryStatusBar } from "./FileLibraryStatusBar";

/**
 * Library-side projection for the Files status bar.
 *
 * The library selection can mean "these loaded rows" or "every row matching the
 * query", so the bar reports the scope explicitly instead of collapsing both
 * into one count. Size is only summed when every selected file is loaded and
 * carries a size; a partial sum would misstate the selection.
 */
export function librarySelectionSize(
  files: readonly { id: string; size?: number }[],
  selection: { kind: "explicit"; fileIds: string[] } | { kind: "all_matching" } | null
): number | null {
  if (selection === null || selection.kind !== "explicit" || selection.fileIds.length === 0) return null;
  const wanted = new Set(selection.fileIds);
  const selected = files.filter((file) => wanted.has(file.id));
  if (selected.length !== wanted.size) return null;
  if (selected.some((file) => file.size === undefined)) return null;
  return selected.reduce((total, file) => total + (file.size ?? 0), 0);
}

export function libraryResultLabels(source: LibrarySourceOwner, t: Translator) {
  const selectionLabel = source.selection?.kind === "all_matching"
    ? replaceCopy(t("librarySelectionAll"), {
      count: source.totalCount === null ? t("libraryCountPending") : source.totalCount.toLocaleString(),
      excluded: source.selection.excludedFileIds.length.toLocaleString()
    })
    : source.selectedIds.size
      ? replaceCopy(t("librarySelectionLoaded"), { count: source.selectedIds.size.toLocaleString() })
      : t("librarySelectionNone");
  const resultCountLabel = source.isLoading
    ? t("loading")
    : source.isCountLoading || source.totalCount === null
      ? replaceCopy(t("libraryResultCountDeferred"), { loaded: source.files.length.toLocaleString() })
      : replaceCopy(t("libraryResultCountExact"), {
        loaded: source.files.length.toLocaleString(),
        total: source.totalCount.toLocaleString()
      });
  return { selectionLabel, resultCountLabel };
}

export function LibraryStatusBar({
  source,
  scopeText,
  t
}: {
  source: LibrarySourceOwner;
  scopeText: string;
  t: Translator;
}) {
  const { selectionLabel, resultCountLabel } = libraryResultLabels(source, t);
  return (
    <>
      <FileLibraryStatusBar
        loadedCount={source.files.length}
        totalCount={source.totalCount ?? null}
        selectionCount={source.selection?.kind === "all_matching"
          ? source.totalCount ?? 0
          : source.selection?.fileIds.length ?? 0}
        selectionScope={source.selection === null
          ? "none"
          : source.selection.kind === "all_matching" ? "all" : "loaded"}
        selectedSize={librarySelectionSize(source.files, source.selection)}
        sourceText={scopeText}
        t={t}
      />
      {/* Assistive technology still hears result changes; the visible counts
       * live in the status bar. */}
      <p className="sr-only" data-library-result-status aria-live="polite">
        {t("libraryResultCountLabel")}: {resultCountLabel} · {t("librarySelectionLabel")}: {selectionLabel}
      </p>
    </>
  );
}
