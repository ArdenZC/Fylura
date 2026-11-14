import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { listColumnSet, listNameMinWidth, listRowHeight, loadedSelectionState } from "../src/views/fileLibrary/list/SharedFileList";
import { statusCountText, statusSelectionText } from "../src/views/fileLibrary/status/FileLibraryStatusBar";
import { librarySelectionCount, librarySelectionSize } from "../src/views/fileLibrary/status/libraryStatus";
import type { PresentationInteractionProjection } from "../src/views/fileLibrary/list/interactionContracts";
import type { LibrarySourceOwner } from "../src/views/fileLibrary/library/librarySourceOwner";
import { gridCellsInRow } from "../src/views/fileLibrary/list/SharedFileGrid";
import { makeTranslator } from "../src/i18n";

const t = makeTranslator("en");

describe("v3.40 Files browser surfaces", () => {
  it("uses the 46/36 row ladder and drops optional columns with their tracks", () => {
    expect(listRowHeight("default")).toBe(46);
    expect(listRowHeight("compact")).toBe(36);
    expect(listRowHeight(undefined)).toBe(46);

    // Wide: type + created + modified + size. Medium: drops type. Narrow:
    // drops created, then modified, and only the name survives at the floor.
    expect(listColumnSet(true, 1200)).toEqual({ showType: true, showCreated: true, showModified: true, showSize: true });
    expect(listColumnSet(true, 850)).toEqual({ showType: false, showCreated: true, showModified: true, showSize: true });
    expect(listColumnSet(true, 650)).toEqual({ showType: false, showCreated: false, showModified: true, showSize: true });
    expect(listColumnSet(true, 500)).toEqual({ showType: false, showCreated: false, showModified: false, showSize: true });
    expect(listColumnSet(true, 400)).toEqual({ showType: false, showCreated: false, showModified: false, showSize: true });
    expect(listColumnSet(true, 340)).toEqual({ showType: false, showCreated: false, showModified: false, showSize: false });
    // The library source has no created column at any width.
    expect(listColumnSet(false, 1200)).toEqual({ showType: true, showCreated: false, showModified: true, showSize: true });
    // The name floor also adapts so a narrow pane never overflows its tracks.
    expect(listNameMinWidth(1200)).toBe(180);
    expect(listNameMinWidth(500)).toBe(150);
    expect(listNameMinWidth(340)).toBe(120);
  });

  it("never reports a loaded page as the directory total", () => {
    expect(statusCountText({ loadedCount: 40, totalCount: 40, t })).toBe("40 items");
    expect(statusCountText({ loadedCount: 40, totalCount: 1200, t })).toBe("Loaded 40 of 1,200");
    // An unknown total is stated as the loaded count only.
    expect(statusCountText({ loadedCount: 40, totalCount: null, t })).toBe("40 items");
  });

  it("separates a loaded-page selection from a whole-query selection", () => {
    expect(statusSelectionText({ selectionCount: 0, selectionScope: "none", selectedSize: null, t })).toBe("No selection");
    expect(statusSelectionText({ selectionCount: 3, selectionScope: "loaded", selectedSize: null, t })).toBe("3 selected");
    expect(statusSelectionText({ selectionCount: 3, selectionScope: "loaded", selectedSize: 2048, t })).toBe("3 selected · 2.0 KB");
    expect(statusSelectionText({ selectionCount: 900, selectionScope: "all", selectedSize: null, t })).toBe("All 900 selected");
    // A whole-query selection whose total is still unpublished is counting,
    // never "nothing selected".
    expect(statusSelectionText({ selectionCount: null, selectionScope: "all", selectedSize: null, t })).toBe("All matches selected · counting");
  });

  it("subtracts exclusions from a whole-query selection and keeps an unknown total unknown", () => {
    const source = (selection: unknown, totalCount: number | null) => ({ selection, totalCount } as unknown as LibrarySourceOwner);
    const exclusions = { kind: "all_matching", query: {}, queryFingerprint: "f", snapshotRevision: 1, excludedFileIds: ["a", "b"] };
    expect(librarySelectionCount(source(exclusions, 100))).toBe(98);
    expect(librarySelectionCount(source(exclusions, null))).toBeNull();
    expect(librarySelectionCount(source({ kind: "explicit", fileIds: ["x", "y"] }, 100))).toBe(2);
    expect(librarySelectionCount(source(null, 100))).toBe(0);
  });

  it("asks the source which loaded rows are selected instead of comparing counts", () => {
    const entries = ["a", "b", "c"].map((id) => ({ entryRef: { fileId: id } }));
    const selected = new Set(["a", "c"]);
    const interaction = {
      source: "library",
      loadedRowCount: 3,
      entryAt: (index: number) => entries[index],
      isSelected: (entry: { entryRef: { fileId: string } }) => selected.has(entry.entryRef.fileId)
    } as unknown as PresentationInteractionProjection;

    // A whole-query selection excluded one loaded row: the box must stay mixed.
    expect(loadedSelectionState(interaction)).toEqual({ selected: 2, all: false, some: true });
    selected.add("b");
    expect(loadedSelectionState(interaction)).toEqual({ selected: 3, all: true, some: false });
  });

  it("only sums a selection size when every selected file is loaded and sized", () => {
    const files = [{ id: "a", size: 10 }, { id: "b", size: 20 }, { id: "c", size: 30 }];
    expect(librarySelectionSize(files, { kind: "explicit", fileIds: ["a", "b"] })).toBe(30);
    // One selected file is outside the loaded page.
    expect(librarySelectionSize(files, { kind: "explicit", fileIds: ["a", "z"] })).toBeNull();
    // A whole-query selection has no honest size.
    expect(librarySelectionSize(files, { kind: "all_matching" })).toBeNull();
    expect(librarySelectionSize(files, null)).toBeNull();
    expect(librarySelectionSize([{ id: "a" }], { kind: "explicit", fileIds: ["a"] })).toBeNull();
  });

  it("keeps the row checkbox non-interactive for assistive tech", () => {
    const list = readFileSync(resolve("src/views/fileLibrary/list/SharedFileList.tsx"), "utf8");
    // The option row owns selection; the visual checkbox must not become a
    // nested control inside role="option".
    expect(list).toContain('className="shared-file-list-select-cell" data-row-select-toggle="true" aria-hidden="true"');
    expect(list).toContain('role="option"');
    expect(list).toContain("aria-selected={selected}");
    // The header control states its scope from the source capability.
    expect(list).toContain('interaction.capabilities.selectAll === "all_matching"');
    expect(list).toContain('t("statusSelectAllMatching")');
    expect(list).toContain('t("statusSelectAllLoaded")');
    // Density changes re-measure the virtualiser instead of keeping stale sizes.
    expect(list).toContain("rowVirtualizer.measure()");
  });

  it("bounds each grid row to the entries it actually has", () => {
    // A complete directory must not render ghost tiles after its last entry.
    expect(gridCellsInRow(0, 7, 4)).toBe(4);
    expect(gridCellsInRow(0, 7, 14)).toBe(7);
    expect(gridCellsInRow(1, 7, 10)).toBe(3);
    expect(gridCellsInRow(2, 7, 14)).toBe(0);
    expect(gridCellsInRow(2, 7, 15)).toBe(1);
    expect(gridCellsInRow(0, 7, 0)).toBe(0);
  });

  it("presents Finder columns from the pane's own navigation stack", () => {
    const view = readFileSync(resolve("src/views/fileLibrary/browse/BrowseColumnView.tsx"), "utf8");
    expect(view).toContain("source.breadcrumbs.map");
    expect(view).toContain("cachedColumns.get(crumb.pathRef.id)");
    expect(view).toContain('t("browseColumnLoadHint")');
    // Selection and pagination still go through the pane's projection; the
    // view never enumerates on its own.
    expect(view).toContain("interaction.actions.select");
    expect(view).toContain("interaction.actions.loadMore");
    expect(view).toContain("interaction.hasMore");
    // Hidden files follow the same display rule as list and grid.
    expect(view).toContain("showHidden ? cached : cached.filter((entry) => !entry.isHidden)");
    // Ancestor activation carries its own column so the chain cannot be
    // appended to the current directory.
    expect(view).toContain("onActivate(entry, isCurrent ? null : crumb)");
    // No second session or enumeration is created here.
    expect(view).not.toContain("browseStartEnumeration");
    expect(view).not.toContain("locationBrowse");
  });

  it("accepts the columns view mode in persisted presentation state", () => {
    const session = readFileSync(resolve("src/fileWorkspace/workspaceSession.ts"), "utf8");
    expect(session).toContain('export type WorkspaceViewMode = "list" | "grid" | "columns";');
    expect(session).toContain('const WORKSPACE_VIEW_MODES = ["list", "grid", "columns"] as const;');
    const experience = readFileSync(resolve("src/views/fileLibrary/fileLibraryExperience.ts"), "utf8");
    expect(experience).toContain('viewMode !== "columns"');
  });
});
