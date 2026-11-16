// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FileWorkspaceController } from "../src/fileWorkspace";
import { makeTranslator } from "../src/i18n";
import type { FileLibrarySummary, LibrarySelectionV1 } from "../src/types/domain";
import type { BrowseEntry } from "../src/types/fileWorkspace";
import { adaptBrowseEntry, adaptLibrarySummary } from "../src/views/fileLibrary/presentation/adapters";
import type { BrowseSourceOwner } from "../src/views/fileLibrary/browse/browseSourceOwner";
import { useBrowseContextMenu } from "../src/views/fileLibrary/browse/useBrowseContextMenu";
import type { LibrarySourceOwner } from "../src/views/fileLibrary/library/librarySourceOwner";
import { FileLibraryContextMenu } from "../src/views/fileLibrary/library/LibraryContextMenu";
import { useLibraryContextMenu } from "../src/views/fileLibrary/library/useLibraryContextMenu";
import {
  createBrowseInteractionProjection,
  createLibraryInteractionProjection
} from "../src/views/fileLibrary/list/interactionAdapters";
import {
  resolveLibraryContextMenuTarget,
  resolvePresentationContextMenuTarget
} from "../src/views/fileLibrary/list/contextMenuTarget";
import { SharedFileGrid } from "../src/views/fileLibrary/list/SharedFileGrid";
import { SharedFileList } from "../src/views/fileLibrary/list/SharedFileList";
import type { PresentationInteractionProjection } from "../src/views/fileLibrary/list/interactionContracts";
import type { PresentationEntry } from "../src/views/fileLibrary/presentation/contracts";

const t = makeTranslator("en");
let root: Root | null = null;
let container: HTMLDivElement | null = null;
const nativeClientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
const nativeClientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
const nativeOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");

function librarySummary(id: string): FileLibrarySummary {
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
    tagCount: 0
  };
}

function browseEntry(id: string): BrowseEntry {
  return {
    ref: { kind: "ephemeral", browseSessionId: "browse-session", entryId: id },
    name: `${id}.txt`,
    displayPath: `${id}.txt`,
    kind: "file",
    materialization: "unknown"
  };
}

function librarySource(files: FileLibrarySummary[], overrides: Record<string, unknown> = {}) {
  return {
    source: "library",
    files,
    totalCount: files.length,
    collection: null,
    focusedId: "",
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
    focusedId: null,
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

function controllerStub() {
  return {
    requestThumbnail: vi.fn(async () => null),
    cancelThumbnail: vi.fn(async () => undefined)
  } as unknown as FileWorkspaceController;
}

function unmount() {
  if (root !== null) {
    act(() => root?.unmount());
    root = null;
  }
  container?.remove();
  container = null;
}

afterEach(() => {
  unmount();
  document.body.innerHTML = "";
  if (nativeClientWidth) Object.defineProperty(HTMLElement.prototype, "clientWidth", nativeClientWidth);
  else delete (HTMLElement.prototype as { clientWidth?: number }).clientWidth;
  if (nativeClientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", nativeClientHeight);
  else delete (HTMLElement.prototype as { clientHeight?: number }).clientHeight;
  if (nativeOffsetHeight) Object.defineProperty(HTMLElement.prototype, "offsetHeight", nativeOffsetHeight);
  else delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
  vi.unstubAllGlobals();
});

async function mountSurface(
  kind: "list" | "grid",
  interaction: PresentationInteractionProjection,
  onOpenContextMenu: (entry: unknown, index: number) => void
) {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  const openContextMenu = onOpenContextMenu as (entry: PresentationEntry, index: number) => void;
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, value: 640 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, value: 240 });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, value: 240 });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);

  await act(async () => {
    if (kind === "list") {
      root!.render(createElement(SharedFileList, {
        interaction,
        language: "en",
        t,
        ariaLabel: "Files",
        onOpenContextMenu: openContextMenu
      }));
    } else {
      root!.render(createElement(SharedFileGrid, {
        interaction,
        language: "en",
        t,
        controller: controllerStub(),
        ariaLabel: "Files",
        onOpenContextMenu: openContextMenu
      }));
    }
    await Promise.resolve();
  });

  const surface = container.querySelector<HTMLElement>(kind === "list" ? '[role="listbox"]' : '[role="grid"]');
  if (!surface) throw new Error(`SharedFile${kind === "list" ? "List" : "Grid"} did not mount`);
  return surface;
}

async function pressShiftF10(surface: HTMLElement) {
  await act(async () => {
    surface.focus();
    surface.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true }));
    await Promise.resolve();
  });
}

const allMatchingSelection = {
  kind: "all_matching",
  query: {} as LibrarySelectionV1 extends { kind: "all_matching"; query: infer Query } ? Query : never,
  queryFingerprint: "query-fingerprint",
  snapshotRevision: 1,
  excludedFileIds: []
} as LibrarySelectionV1;

describe("W2-10 context-menu target and dismissal behavior", () => {
  it("fails closed for Library and Browse with no logical focus or selected loaded entry", async () => {
    for (const kind of ["list", "grid"] as const) {
      const libraryFiles = [librarySummary("library-1"), librarySummary("library-2")];
      const librarySetExplicitSelection = vi.fn();
      const library = librarySource(libraryFiles, { setExplicitSelection: librarySetExplicitSelection });
      const libraryOpen = vi.fn();
      const librarySurface = await mountSurface(kind, createLibraryInteractionProjection(library), libraryOpen);
      await pressShiftF10(librarySurface);
      expect(libraryOpen).not.toHaveBeenCalled();
      expect(librarySetExplicitSelection).not.toHaveBeenCalled();
      unmount();

      const browseEntries = ["browse-1", "browse-2"].map((id) => adaptBrowseEntry(browseEntry(id)));
      const browseSelectEntry = vi.fn();
      const browse = browseSource(browseEntries, { selectEntry: browseSelectEntry });
      const browseOpen = vi.fn();
      const browseSurface = await mountSurface(kind, createBrowseInteractionProjection(browse), browseOpen);
      await pressShiftF10(browseSurface);
      expect(browseOpen).not.toHaveBeenCalled();
      expect(browseSelectEntry).not.toHaveBeenCalled();
      unmount();
    }
  });

  it("does not manufacture a loaded Library row from all_matching", () => {
    const files = [librarySummary("library-1"), librarySummary("library-2")];
    const target = resolveLibraryContextMenuTarget({
      files,
      focusedId: "",
      selection: allMatchingSelection
    });
    expect(target).toBeNull();
  });

  it("targets the exact focused entry identically in List and Grid", async () => {
    for (const kind of ["list", "grid"] as const) {
      const libraryFiles = [librarySummary("library-1"), librarySummary("library-2")];
      const library = librarySource(libraryFiles, { focusedId: "library-2" });
      const libraryOpen = vi.fn();
      const librarySurface = await mountSurface(kind, createLibraryInteractionProjection(library), libraryOpen);
      await pressShiftF10(librarySurface);
      expect(libraryOpen).toHaveBeenCalledWith(expect.objectContaining({ entryRef: expect.objectContaining({ fileId: "library-2" }) }), 1);
      unmount();

      const browseEntries = ["browse-1", "browse-2"].map((id) => adaptBrowseEntry(browseEntry(id)));
      const browse = browseSource(browseEntries, { focusedId: "browse-2" });
      const browseOpen = vi.fn();
      const browseSurface = await mountSurface(kind, createBrowseInteractionProjection(browse), browseOpen);
      await pressShiftF10(browseSurface);
      expect(browseOpen).toHaveBeenCalledWith(expect.objectContaining({ entryRef: expect.objectContaining({ entryId: "browse-2" }) }), 1);
      unmount();
    }
  });

  it("targets one explicit selected loaded entry when logical focus is absent", async () => {
    for (const kind of ["list", "grid"] as const) {
      const libraryFiles = [librarySummary("library-1"), librarySummary("library-2")];
      const library = librarySource(libraryFiles, {
        selection: { kind: "explicit", fileIds: ["library-2"] },
        selectionContainsFileId: vi.fn((fileId: string) => fileId === "library-2")
      });
      const libraryOpen = vi.fn();
      const librarySurface = await mountSurface(kind, createLibraryInteractionProjection(library), libraryOpen);
      await pressShiftF10(librarySurface);
      expect(libraryOpen).toHaveBeenCalledWith(expect.objectContaining({ entryRef: expect.objectContaining({ fileId: "library-2" }) }), 1);
      unmount();

      const browseEntries = ["browse-1", "browse-2"].map((id) => adaptBrowseEntry(browseEntry(id)));
      const browse = browseSource(browseEntries, { selectedIds: new Set(["browse-2"]) });
      const browseOpen = vi.fn();
      const browseSurface = await mountSurface(kind, createBrowseInteractionProjection(browse), browseOpen);
      await pressShiftF10(browseSurface);
      expect(browseOpen).toHaveBeenCalledWith(expect.objectContaining({ entryRef: expect.objectContaining({ entryId: "browse-2" }) }), 1);
      unmount();
    }
  });

  it("opens focused keyboard targets without mutating Library or Browse selection", async () => {
    const librarySetExplicitSelection = vi.fn();
    const library = librarySource(
      [librarySummary("library-1"), librarySummary("library-2")],
      { focusedId: "library-2", setExplicitSelection: librarySetExplicitSelection }
    );
    const browseSelectEntry = vi.fn();
    const browse = browseSource(
      ["browse-1", "browse-2"].map((id) => adaptBrowseEntry(browseEntry(id))),
      { focusedId: "browse-2", selectEntry: browseSelectEntry }
    );

    function Harness() {
      const libraryMenu = useLibraryContextMenu({ source: library, restoreFocus: vi.fn() });
      const browseMenu = useBrowseContextMenu({ source: browse, restoreFocus: vi.fn() });
      return createElement(
        "div",
        null,
        createElement("button", { "data-open-library": true, onClick: libraryMenu.openFocusedContextMenu }, "Library"),
        createElement("button", { "data-open-browse": true, onClick: browseMenu.openFocusedContextMenu }, "Browse"),
        libraryMenu.contextMenu
          ? createElement("output", { "data-library-target": true }, libraryMenu.contextMenu.file.name)
          : null,
        browseMenu.contextMenu
          ? createElement("output", { "data-browse-target": true }, browseMenu.contextMenu.entry.displayName)
          : null
      );
    }

    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => root!.render(createElement(Harness)));
    await act(async () => container!.querySelector<HTMLElement>("[data-open-library]")!.click());
    expect(container.querySelector("[data-library-target]")?.textContent).toBe("library-2.txt");
    expect(librarySetExplicitSelection).not.toHaveBeenCalled();

    await act(async () => container!.querySelector<HTMLElement>("[data-open-browse]")!.click());
    expect(container.querySelector("[data-browse-target]")?.textContent).toBe("browse-2.txt");
    expect(browseSelectEntry).not.toHaveBeenCalled();
  });

  it("keeps Escape as one menu owner and restores focus once", async () => {
    const files = [librarySummary("library-1")];
    const source = librarySource(files, {
      focusedId: "library-1",
      selection: { kind: "explicit", fileIds: ["library-1"] },
      selectionContainsFileId: vi.fn(() => true)
    });
    const restoreFocus = vi.fn((target: HTMLElement | null) => target?.focus());
    const rafCallbacks: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      rafCallbacks.push(callback);
      return rafCallbacks.length;
    });

    function Harness() {
      const menu = useLibraryContextMenu({ source, restoreFocus });
      return createElement(
        "div",
        { "data-library-source-owner": "query-v2" },
        createElement("div", { "data-shared-file-list-source": "library", tabIndex: 0 }),
        createElement("button", { "data-open-menu": true, onClick: menu.openFocusedContextMenu }, "Open"),
        menu.contextMenu
          ? createElement(FileLibraryContextMenu, {
              x: 8,
              y: 8,
              title: menu.contextMenu.file.name,
              ariaLabel: "Library menu",
              items: [{ label: "Action", action: vi.fn() }],
              onClose: () => menu.closeContextMenu("action")
            })
          : null
      );
    }

    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => root!.render(createElement(Harness)));
    const trigger = container.querySelector<HTMLElement>("[data-open-menu]")!;
    await act(async () => trigger.click());
    const menuItem = container.querySelector<HTMLElement>('[role="menuitem"]');
    expect(menuItem).not.toBeNull();

    await act(async () => {
      menuItem!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await Promise.resolve();
    });
    expect(container.querySelector('[role="menu"]')).toBeNull();
    expect(rafCallbacks).toHaveLength(1);
    await act(async () => {
      rafCallbacks.shift()?.(0);
      await Promise.resolve();
    });
    expect(restoreFocus).toHaveBeenCalledTimes(1);
  });

  it("does not steal focus from Search after outside-pointer dismissal", async () => {
    const files = [librarySummary("library-1")];
    const source = librarySource(files, {
      focusedId: "library-1",
      selection: { kind: "explicit", fileIds: ["library-1"] },
      selectionContainsFileId: vi.fn(() => true)
    });
    const restoreFocus = vi.fn((target: HTMLElement | null) => target?.focus());
    const rafCallbacks: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      rafCallbacks.push(callback);
      return rafCallbacks.length;
    });

    function Harness() {
      const menu = useLibraryContextMenu({ source, restoreFocus });
      return createElement(
        "div",
        { "data-library-source-owner": "query-v2" },
        createElement("div", { "data-shared-file-list-source": "library", tabIndex: 0 }),
        createElement("input", { "data-search": true }),
        createElement("button", { "data-open-menu": true, onClick: menu.openFocusedContextMenu }, "Open"),
        menu.contextMenu
          ? createElement(FileLibraryContextMenu, {
              x: 8,
              y: 8,
              title: menu.contextMenu.file.name,
              ariaLabel: "Library menu",
              items: [{ label: "Action", action: vi.fn() }],
              onClose: () => menu.closeContextMenu("action")
            })
          : null
      );
    }

    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    await act(async () => root!.render(createElement(Harness)));
    await act(async () => container!.querySelector<HTMLElement>("[data-open-menu]")!.click());
    const search = container.querySelector<HTMLInputElement>("[data-search]")!;
    await act(async () => {
      search.focus();
      search.dispatchEvent(new Event("pointerdown", { bubbles: true }));
      await Promise.resolve();
    });
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 160)); });
    expect(container.querySelector('[role="menu"]')).toBeNull();
    expect(rafCallbacks).toHaveLength(1);
    await act(async () => {
      rafCallbacks.shift()?.(0);
      await Promise.resolve();
    });
    expect(document.activeElement).toBe(search);
    expect(restoreFocus).not.toHaveBeenCalled();
  });

  it("uses the same resolver result for the rendered projection and source hook inputs", () => {
    const files = [librarySummary("library-1"), librarySummary("library-2")];
    const source = librarySource(files, { focusedId: "" });
    const projection = createLibraryInteractionProjection(source);
    expect(resolvePresentationContextMenuTarget(projection)).toBeNull();
    expect(resolveLibraryContextMenuTarget({ files, focusedId: "", selection: null })).toBeNull();
  });
});
