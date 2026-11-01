// @vitest-environment happy-dom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import type { FileLibrarySummary } from "../src/types/domain";
import type { BrowseEntry } from "../src/types/fileWorkspace";
import { adaptBrowseEntry, adaptLibrarySummary } from "../src/views/fileLibrary/presentation/adapters";
import type { BrowseSourceOwner } from "../src/views/fileLibrary/browse/browseSourceOwner";
import type { LibrarySourceOwner } from "../src/views/fileLibrary/library/librarySourceOwner";
import {
  createBrowseInteractionProjection,
  createLibraryInteractionProjection,
  selectionIntentFromModifiers
} from "../src/views/fileLibrary/list/interactionAdapters";
import { decideListLoadMore, nextNavigationIndex, SharedFileList } from "../src/views/fileLibrary/list/SharedFileList";

const t = makeTranslator("en");
const SCROLL_SETTLE_MS = 200;
let root: Root | null = null;
let container: HTMLDivElement | null = null;
const nativeClientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
const nativeOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");

function librarySummary(id: string, overrides: Partial<FileLibrarySummary> = {}): FileLibrarySummary {
  return {
    id,
    name: `${id}.txt`,
    extension: "txt",
    displayDirectory: "C:/Library",
    size: 12,
    modifiedAt: 7,
    createdAt: 6,
    isDirectory: false,
    fileType: "Document",
    purpose: "Work",
    lifecycle: "Active",
    risk: "Low",
    confidence: 1,
    isDuplicate: false,
    requiresReview: false,
    isStale: false,
    tags: [],
    tagCount: 0,
    ...overrides
  };
}

function browseEntry(id: string, kind: BrowseEntry["kind"] = "file"): BrowseEntry {
  return {
    ref: { kind: "ephemeral", browseSessionId: "browse-session", entryId: id },
    pathRef: kind === "directory" ? { id: `path-${id}` } : undefined,
    name: `${id}.txt`,
    displayPath: `${id}.txt`,
    kind,
    materialization: "unknown"
  };
}

function librarySource(files: FileLibrarySummary[], overrides: Record<string, unknown> = {}) {
  return {
    source: "library",
    files,
    totalCount: files.length,
    collection: null,
    focusedId: files[0]?.id ?? "",
    selection: null,
    selectionContainsFileId: vi.fn(() => false),
    hasMore: false,
    isLoading: false,
    loadNextPage: vi.fn(async () => undefined),
    setExplicitSelection: vi.fn(),
    toggleSelection: vi.fn(),
    selectAllMatching: vi.fn(),
    clearSelection: vi.fn(),
    ...overrides
  } as unknown as LibrarySourceOwner;
}

function browseSource(entries: ReturnType<typeof adaptBrowseEntry>[], overrides: Record<string, unknown> = {}) {
  return {
    entries,
    collection: null,
    focusedId: entries[0]?.entryRef.entryId ?? null,
    selectedIds: new Set<string>(),
    hasMore: false,
    enumerationState: "complete",
    loadNextPage: vi.fn(async () => undefined),
    selectEntry: vi.fn(),
    selectAllLoaded: vi.fn(),
    clearSelection: vi.fn(),
    setFocusedId: vi.fn(),
    ...overrides
  } as unknown as BrowseSourceOwner;
}

afterEach(async () => {
  if (root !== null) {
    await act(async () => {
      await new Promise<void>((resolve) => window.setTimeout(resolve, SCROLL_SETTLE_MS));
    });
    act(() => root?.unmount());
    root = null;
  }
  container?.remove();
  container = null;
  document.body.innerHTML = "";
  if (nativeClientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", nativeClientHeight);
  else delete (HTMLElement.prototype as { clientHeight?: number }).clientHeight;
  if (nativeOffsetHeight) Object.defineProperty(HTMLElement.prototype, "offsetHeight", nativeOffsetHeight);
  else delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
  vi.unstubAllGlobals();
});

async function mountSharedList(interaction: ReturnType<typeof createBrowseInteractionProjection> | ReturnType<typeof createLibraryInteractionProjection>) {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, value: 176 });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, value: 176 });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);

  await act(async () => {
    root!.render(createElement(SharedFileList, {
      interaction,
      language: "en",
      t,
      ariaLabel: "Files"
    }));
    await Promise.resolve();
  });

  const list = container.querySelector<HTMLElement>('[role="listbox"]');
  if (!list) throw new Error("SharedFileList did not mount a listbox");
  await act(async () => {
    list.dispatchEvent(new Event("scroll", { bubbles: true }));
    await Promise.resolve();
  });
  return list;
}

describe("W2-05 interaction convergence", () => {
  it("keeps Library and Browse projections discriminated and source-bound", () => {
    const library = createLibraryInteractionProjection(librarySource([librarySummary("library-1")]));
    const browse = createBrowseInteractionProjection(browseSource([adaptBrowseEntry(browseEntry("browse-1"))]));

    expect(library.source).toBe("library");
    expect(library.capabilities.selectAll).toBe("all_matching");
    expect(browse.source).toBe("browse");
    expect(browse.capabilities.selectAll).toBe("loaded");
    expect(library.entryAt(0)?.source).toBe("library");
    expect(browse.entryAt(0)?.source).toBe("browse");
    expect(selectionIntentFromModifiers({ shiftKey: true, metaKey: false, ctrlKey: false })).toBe("range");
    expect(selectionIntentFromModifiers({ shiftKey: false, metaKey: true, ctrlKey: false })).toBe("toggle");
  });

  it("routes Library Ctrl/Cmd+A to compact all_matching without logical ID materialization", () => {
    const files = [librarySummary("library-1")];
    const source = librarySource(files, { totalCount: 100_000, hasMore: true });
    const projection = createLibraryInteractionProjection(source);

    expect(projection.rowCount).toBe(100_000);
    expect(projection.loadedRowCount).toBe(1);
    projection.actions.selectAll();

    expect(source.selectAllMatching).toHaveBeenCalledTimes(1);
    expect(source.setExplicitSelection).not.toHaveBeenCalled();
    expect(source.toggleSelection).not.toHaveBeenCalled();
    expect(projection.entryAt(99_999)).toBeUndefined();
  });

  it("routes Browse Select All to loaded entries only and preserves source-local range/toggle", () => {
    const entries = ["browse-1", "browse-2", "browse-3"].map((id) => adaptBrowseEntry(browseEntry(id)));
    const source = browseSource(entries, { hasMore: true, enumerationState: "partial" });
    const projection = createBrowseInteractionProjection(source);

    expect(projection.rowCount).toBe(3);
    projection.actions.selectAll();
    projection.actions.select(entries[1]!, 1, "toggle");
    projection.actions.select(entries[2]!, 2, "range");

    expect(source.selectAllLoaded).toHaveBeenCalledTimes(1);
    expect(source.selectEntry).toHaveBeenNthCalledWith(1, "browse-2", "toggle");
    expect(source.selectEntry).toHaveBeenNthCalledWith(2, "browse-3", "range");
  });

  it("keeps focus projection independent from mounted row membership", () => {
    const entries = ["browse-1", "browse-2"].map((id) => adaptBrowseEntry(browseEntry(id)));
    const source = browseSource(entries, { focusedId: "browse-2" });
    const projection = createBrowseInteractionProjection(source);

    expect(projection.focusedIndex).toBe(1);
    projection.actions.focus(entries[1]!, 1);
    expect(source.setFocusedId).toHaveBeenCalledWith("browse-2");
    expect(projection.entryAt(1)?.renderKey).toContain("browse-2");
  });

  it("keeps logical focus source-owned while a manually scrolled row unmounts and remounts", async () => {
    const entries = Array.from({ length: 100 }, (_, index) =>
      adaptBrowseEntry(browseEntry(`browse-${index}`))
    );
    const source = browseSource(entries, { focusedId: "browse-0" });
    const list = await mountSharedList(createBrowseInteractionProjection(source));
    const focusedRowId = "browse-row-browse-0";

    expect(list.querySelector(`#${focusedRowId}`)).not.toBeNull();
    expect(list.getAttribute("aria-activedescendant")).toBe(focusedRowId);

    await act(async () => {
      list.scrollTop = 44 * 30;
      list.dispatchEvent(new Event("scroll", { bubbles: true }));
      await Promise.resolve();
    });

    expect(list.scrollTop).toBeGreaterThan(0);
    expect(list.querySelector(`#${focusedRowId}`)).toBeNull();
    expect(source.focusedId).toBe("browse-0");
    expect(list.getAttribute("aria-activedescendant")).toBeNull();

    await act(async () => {
      list.scrollTop = 0;
      list.dispatchEvent(new Event("scroll", { bubbles: true }));
      await Promise.resolve();
    });

    expect(list.querySelector(`#${focusedRowId}`)?.classList.contains("is-focused")).toBe(true);
    expect(list.getAttribute("aria-activedescendant")).toBe(focusedRowId);
  });

  it("defines bounded no-focus keyboard destinations instead of relying on -1 arithmetic", async () => {
    const entries = Array.from({ length: 8 }, (_, index) =>
      adaptBrowseEntry(browseEntry(`browse-${index}`))
    );
    const setFocusedId = vi.fn();
    const source = browseSource(entries, { focusedId: null, setFocusedId });
    const list = await mountSharedList(createBrowseInteractionProjection(source));

    expect(nextNavigationIndex("ArrowDown", -1, 8, list)).toBe(0);
    expect(nextNavigationIndex("ArrowUp", -1, 8, list)).toBe(0);
    expect(nextNavigationIndex("Home", -1, 8, list)).toBe(0);
    expect(nextNavigationIndex("End", -1, 8, list)).toBe(7);
    expect(nextNavigationIndex("PageUp", -1, 8, list)).toBe(0);
    expect(nextNavigationIndex("PageDown", -1, 8, list)).toBe(3);
    expect(nextNavigationIndex("ArrowUp", 0, 8, list)).toBe(0);
    expect(nextNavigationIndex("ArrowDown", 7, 8, list)).toBe(7);
    expect(nextNavigationIndex("PageUp", 0, 8, list)).toBe(0);
    expect(nextNavigationIndex("PageDown", 7, 8, list)).toBe(7);

    for (const [key, expectedId] of [
      ["ArrowDown", "browse-0"],
      ["ArrowUp", "browse-0"],
      ["Home", "browse-0"],
      ["End", "browse-7"],
      ["PageUp", "browse-0"],
      ["PageDown", "browse-3"]
    ] as const) {
      setFocusedId.mockClear();
      await act(async () => {
        list.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
        await Promise.resolve();
      });
      expect(setFocusedId).toHaveBeenCalledWith(expectedId);
    }
  });

  it("clamps far exact-count list jumps while allowing a near-end page request", () => {
    expect(decideListLoadMore({
      source: "library",
      hasMore: true,
      isLoadingMore: false,
      loadedRowCount: 50,
      lastVisibleIndex: 100,
      scrollTop: 4_400
    })).toEqual({ kind: "clamp", rowIndex: 49 });

    expect(decideListLoadMore({
      source: "library",
      hasMore: true,
      isLoadingMore: false,
      loadedRowCount: 50,
      lastVisibleIndex: 53,
      scrollTop: 2_200
    })).toEqual({ kind: "load" });

    expect(decideListLoadMore({
      source: "library",
      hasMore: true,
      isLoadingMore: false,
      loadedRowCount: 50,
      lastVisibleIndex: 20,
      scrollTop: 880
    })).toEqual({ kind: "none" });
  });

  it("routes Shift-click to the source-owned range action", async () => {
    const entries = ["browse-0", "browse-1", "browse-2"].map((id) => adaptBrowseEntry(browseEntry(id)));
    const selectEntry = vi.fn();
    const source = browseSource(entries, { selectEntry });
    await mountSharedList(createBrowseInteractionProjection(source));

    const row = container!.querySelector<HTMLElement>('[data-browse-entry-id="browse-2"]');
    if (!row) throw new Error("Browse row did not mount");
    await act(async () => {
      row.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: true }));
      await Promise.resolve();
    });

    expect(selectEntry).toHaveBeenCalledWith("browse-2", "range");
  });

  it("projects Library stale summaries as missing rows without changing normal rows", async () => {
    const stale = librarySummary("library-stale", { isStale: true });
    const normal = librarySummary("library-normal", { isStale: false });
    const projection = createLibraryInteractionProjection(librarySource([stale, normal]));
    await mountSharedList(projection);

    const staleRow = container!.querySelector<HTMLElement>('[data-library-row="library-stale"]');
    const normalRow = container!.querySelector<HTMLElement>('[data-library-row="library-normal"]');
    expect(staleRow?.classList.contains("is-missing")).toBe(true);
    expect(staleRow?.textContent).toContain(t("libraryFileNotFound"));
    expect(staleRow?.getAttribute("aria-label")).toContain(t("libraryFileNotFound"));
    expect(normalRow?.classList.contains("is-missing")).toBe(false);
    expect(normalRow?.textContent).not.toContain(t("libraryFileNotFound"));
  });

  it("keeps the final Library/Browse render surface shared and renderKey presentation-only", () => {
    const libraryMode = readFileSync(resolve("src/views/fileLibrary/library/LibraryMode.tsx"), "utf8");
    const browseMode = readFileSync(resolve("src/views/fileLibrary/browse/BrowseMode.tsx"), "utf8");
    const list = readFileSync(resolve("src/views/fileLibrary/list/SharedFileList.tsx"), "utf8");

    expect(libraryMode).toContain("SharedFileList");
    expect(browseMode).toContain("SharedFileList");
    expect(libraryMode).not.toContain("<FileLibraryList");
    expect(browseMode).not.toContain("BrowseEntryList");
    expect(list).toContain("key={entry.renderKey}");
    expect(list).not.toContain("parseRenderKey");
    expect(list).not.toContain("entry.renderKey.split");
    expect(list).toContain("entryAt(virtualRow.index)");
  });

  it("mounts a bounded virtual row window for a large logical projection", async () => {
    class ResizeObserverStub {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", ResizeObserverStub);
    const files = [librarySummary("library-1"), librarySummary("library-2")];
    const source = librarySource(files, { totalCount: 100_000 });
    const projection = createLibraryInteractionProjection(source);
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    await act(async () => {
      root!.render(createElement(SharedFileList, {
        interaction: projection,
        language: "en",
        t,
        ariaLabel: "Files"
      }));
      await Promise.resolve();
    });

    const list = container.querySelector('[role="listbox"]');
    expect(list?.getAttribute("data-file-library-logical-count")).toBe("100000");
    expect(container.querySelectorAll('[role="option"]').length).toBeLessThan(100);
    vi.unstubAllGlobals();
  });
});
