// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { BrowseColumnView, COLUMN_ROW_WINDOW } from "../src/views/fileLibrary/browse/BrowseColumnView";
import type { BrowseBreadcrumb, BrowseSourceOwner } from "../src/views/fileLibrary/browse/browseSourceOwner";
import type { BrowseInteractionProjection } from "../src/views/fileLibrary/list/interactionContracts";
import type { BrowsePresentationEntry } from "../src/views/fileLibrary/presentation/contracts";
import { makeTranslator } from "../src/i18n";

const t = makeTranslator("en");
const rootCrumb: BrowseBreadcrumb = { sessionId: "s1", pathRef: { id: "root" }, label: "Root" };
const childCrumb: BrowseBreadcrumb = { sessionId: "s1", pathRef: { id: "child" }, label: "Child" };

function entry(
  id: string,
  name: string,
  kind: "file" | "directory" = "file",
  extra: { isHidden?: boolean } = {}
): BrowsePresentationEntry {
  return {
    source: "browse",
    renderKey: `render-${id}` as BrowsePresentationEntry["renderKey"],
    entryKind: kind,
    displayName: name,
    entryRef: { kind: "ephemeral", browseSessionId: "s1", entryId: id },
    pathRef: kind === "directory" ? { id: `path-${id}` } : undefined,
    isHidden: extra.isHidden
  };
}

function makeInteraction(
  entries: readonly BrowsePresentationEntry[],
  overrides: Partial<BrowseInteractionProjection> = {}
) {
  const selected = new Set<string>();
  const base = {
    source: "browse",
    collection: null,
    rowCount: entries.length,
    loadedRowCount: entries.length,
    entryAt: (index: number) => entries[index],
    focusedIndex: -1,
    isSelected: () => false,
    isFocused: () => false,
    hasMore: false,
    isLoadingMore: false,
    selection: selected,
    capabilities: { selectAll: "loaded", canActivate: true, canLoadMore: false },
    actions: {
      select: vi.fn(),
      selectAll: vi.fn(),
      clearSelection: vi.fn(),
      focus: vi.fn(),
      loadMore: vi.fn()
    }
  };
  return { ...base, ...overrides } as unknown as BrowseInteractionProjection;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  Element.prototype.scrollIntoView = vi.fn();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

function renderView({
  current,
  cached,
  showHidden = false,
  interaction,
  loadedCount = current.length,
  knownCount = null,
  onActivate = vi.fn(),
  onNavigateCrumb = vi.fn(),
  onPreview = vi.fn(),
  onContextMenu = vi.fn(),
  onOpenContextMenu = vi.fn()
}: {
  current: readonly BrowsePresentationEntry[];
  cached?: readonly BrowsePresentationEntry[];
  showHidden?: boolean;
  interaction: BrowseInteractionProjection;
  loadedCount?: number;
  knownCount?: number | null;
  onActivate?: Mock<(entry: BrowsePresentationEntry, ancestor: BrowseBreadcrumb | null) => void>;
  onNavigateCrumb?: Mock<(index: number) => void>;
  onPreview?: Mock<(entry: BrowsePresentationEntry) => void>;
  onContextMenu?: Mock<(event: unknown, entry: BrowsePresentationEntry) => void>;
  onOpenContextMenu?: Mock<(entry: BrowsePresentationEntry, index: number) => void>;
}) {
  const source = {
    breadcrumbs: [rootCrumb, childCrumb],
    entries: current,
    selectedIds: new Set<string>(),
    focusedId: null,
    loadedCount,
    knownCount
  } as unknown as BrowseSourceOwner;
  const cachedColumns = new Map<string, readonly BrowsePresentationEntry[]>();
  if (cached !== undefined) cachedColumns.set("root", cached);

  act(() => {
    root.render(createElement(BrowseColumnView, {
      source,
      interaction,
      t,
      cachedColumns,
      showHidden,
      onActivate,
      onNavigateCrumb,
      onPreview,
      onContextMenu,
      onOpenContextMenu
    }));
  });
  return { source, onActivate, onNavigateCrumb, onPreview, onContextMenu, onOpenContextMenu };
}

function keydown(element: Element, key: string, init: KeyboardEventInit = {}) {
  act(() => {
    element.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));
  });
}

describe("Browse column view interaction", () => {
  it("honours the hidden-file setting in the live and ancestor columns", () => {
    const current = [entry("c1", "Visible.txt"), entry("c2", ".env", "file", { isHidden: true })];
    const cached = [entry("r1", "Sibling", "directory"), entry("r2", ".secret", "file", { isHidden: true })];

    const interaction = makeInteraction(current);
    renderView({ current, cached, interaction });

    expect(container.textContent).toContain("Visible.txt");
    expect(container.textContent).not.toContain(".env");
    expect(container.textContent).toContain("Sibling");
    expect(container.textContent).not.toContain(".secret");

    renderView({ current, cached, interaction, showHidden: true });
    expect(container.textContent).toContain(".env");
    expect(container.textContent).toContain(".secret");
  });

  it("loads one owner page at a time and reports loaded vs known totals", () => {
    const current = Array.from({ length: 32 }, (_, index) => entry(`c${index}`, `file-${index}.txt`));
    const loadMore = vi.fn();
    const interaction = makeInteraction(current, {
      hasMore: true,
      capabilities: { selectAll: "loaded", canActivate: true, canLoadMore: true },
      actions: {
        select: vi.fn(), selectAll: vi.fn(), clearSelection: vi.fn(), focus: vi.fn(), loadMore
      }
    });

    renderView({ current, interaction, loadedCount: 32, knownCount: 1200 });

    expect(container.querySelector(".browse-column-count")?.textContent).toBe("32 / 1,200");
    act(() => {
      container.querySelector<HTMLButtonElement>(".browse-column-load-more")?.click();
    });
    expect(loadMore).toHaveBeenCalledTimes(1);
    // Pagination is the source's action: the view never enumerates on its own.
    expect(container.querySelectorAll(".browse-column-row").length).toBe(32);
  });

  it("bounds the rendered rows and reveals them in windows", () => {
    const current = Array.from({ length: COLUMN_ROW_WINDOW + 40 }, (_, index) => entry(`c${index}`, `file-${index}.txt`));
    renderView({ current, interaction: makeInteraction(current) });

    expect(container.querySelectorAll(".browse-column-row").length).toBe(COLUMN_ROW_WINDOW);
    const showMore = container.querySelector<HTMLButtonElement>(".browse-column-show-more");
    expect(showMore).not.toBeNull();
    act(() => showMore?.click());
    expect(container.querySelectorAll(".browse-column-row").length).toBe(COLUMN_ROW_WINDOW + 40);
  });

  it("anchors an ancestor folder activation to its own column", () => {
    const current = [entry("c1", "Child.txt")];
    const sibling = entry("r1", "Sibling", "directory");
    const cached = [sibling];
    const { onActivate } = renderView({ current, cached, interaction: makeInteraction(current) });

    act(() => {
      container.querySelector<HTMLButtonElement>('[data-browse-column-ancestor] button.browse-column-row')?.click();
    });

    expect(onActivate).toHaveBeenCalledWith(sibling, rootCrumb);
    // The current column does not claim the ancestor activation.
    expect(onActivate).not.toHaveBeenCalledWith(sibling, null);
  });

  it("keeps ancestor files inert instead of acting on the current directory's selection", () => {
    const current = [entry("c1", "Child.txt")];
    const cached = [entry("r1", ".secret.txt")];
    const { onActivate } = renderView({ current, cached, interaction: makeInteraction(current) });

    const inert = container.querySelector<HTMLElement>('[data-browse-column-ancestor] .browse-column-row.is-static');
    expect(inert).not.toBeNull();
    expect(inert?.closest("button")).toBeNull();
    act(() => inert?.click());
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("moves focus with the arrow keys and opens the context menu for the focused row", () => {
    const current = [entry("c1", "one.txt"), entry("c2", "two.txt")];
    const focus = vi.fn();
    const select = vi.fn();
    const interaction = makeInteraction(current, {
      actions: { select, selectAll: vi.fn(), clearSelection: vi.fn(), focus, loadMore: vi.fn() }
    });
    const { onOpenContextMenu } = renderView({ current, interaction });

    const first = container.querySelector<HTMLElement>('[data-column-row-index="0"]')!;
    keydown(first, "ArrowDown");
    expect(focus).toHaveBeenCalledWith(current[1], 1);
    expect(document.activeElement).toBe(container.querySelector('[data-column-row-index="1"]'));

    keydown(container.querySelector('[data-column-row-index="1"]')!, "ContextMenu");
    expect(select).toHaveBeenCalledWith(current[1], 1, "replace");
    expect(onOpenContextMenu).toHaveBeenCalledWith(current[1], 1);
  });

  it("routes Space to the shared Quick Preview controller", () => {
    const current = [entry("c1", "one.txt"), entry("c2", "two.txt")];
    const { onPreview } = renderView({ current, interaction: makeInteraction(current) });

    keydown(container.querySelector('[data-column-row-index="0"]')!, " ");
    expect(onPreview).toHaveBeenCalledTimes(1);
    expect((onPreview.mock.calls[0] as unknown[])[0]).toBe(current[0]);
  });

  it("opens the folder from the current column and selects files there", () => {
    const folder = entry("c1", "Docs", "directory");
    const file = entry("c2", "report.pdf");
    const current = [folder, file];
    const select = vi.fn();
    const interaction = makeInteraction(current, {
      actions: { select, selectAll: vi.fn(), clearSelection: vi.fn(), focus: vi.fn(), loadMore: vi.fn() }
    });
    const { onActivate } = renderView({ current, interaction });

    act(() => {
      container.querySelector<HTMLButtonElement>('[data-column-row-index="0"]')?.click();
    });
    expect(onActivate).toHaveBeenCalledWith(folder, null);

    act(() => {
      container.querySelector<HTMLButtonElement>('[data-column-row-index="1"]')?.click();
    });
    expect(select).toHaveBeenCalledWith(file, 1, "replace");
    expect(onActivate).toHaveBeenCalledTimes(1);
  });
});
