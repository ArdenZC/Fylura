import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { listColumnSet, listRowHeight } from "../src/views/fileLibrary/list/SharedFileList";
import { statusCountText, statusSelectionText } from "../src/views/fileLibrary/status/FileLibraryStatusBar";
import { librarySelectionSize } from "../src/views/fileLibrary/status/libraryStatus";
import { makeTranslator } from "../src/i18n";

const t = makeTranslator("en");

describe("v3.40 Files browser surfaces", () => {
  it("uses the 46/36 row ladder and drops optional columns with their tracks", () => {
    expect(listRowHeight("default")).toBe(46);
    expect(listRowHeight("compact")).toBe(36);
    expect(listRowHeight(undefined)).toBe(46);

    // Wide: type + created. Medium: created only. Narrow: neither.
    expect(listColumnSet(true, 1200)).toEqual({ showType: true, showCreated: true });
    expect(listColumnSet(true, 850)).toEqual({ showType: false, showCreated: true });
    expect(listColumnSet(true, 600)).toEqual({ showType: false, showCreated: false });
    // The library source has no created column at any width.
    expect(listColumnSet(false, 1200)).toEqual({ showType: true, showCreated: false });
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

  it("presents Finder columns from the pane's own navigation stack", () => {
    const view = readFileSync(resolve("src/views/fileLibrary/browse/BrowseColumnView.tsx"), "utf8");
    expect(view).toContain("source.breadcrumbs.map");
    expect(view).toContain("cachedColumns.get(crumb.pathRef.id)");
    expect(view).toContain('t("browseColumnLoadHint")');
    // Selection still goes through the pane's interaction projection.
    expect(view).toContain('interaction.source === "browse"');
    expect(view).toContain("interaction.actions.select");
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
