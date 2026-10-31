// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fileWorkspaceApi } from "../src/api/fileWorkspaceApi";
import { makeTranslator } from "../src/i18n";
import { DirectoryTree } from "../src/views/fileLibrary/navigation/DirectoryTree";
import { MAX_PERSISTED_TREE_PATHS, readExpandedTreePaths, writeExpandedTreePaths } from "../src/views/fileLibrary/workbench/workbenchPersistence";
import type { BrowseEntry, BrowsePage, LocationDescriptor } from "../src/types/fileWorkspace";
import type { FileLibraryExperienceController } from "../src/views/fileLibrary/fileLibraryExperience";

const t = makeTranslator("en");
const location = {
  ref: { kind: "ephemeral", browseSessionId: "visible-session", locationId: "location" },
  displayName: "C:/",
  kind: "local",
  availability: "available",
  freshness: "current",
  capabilities: { canBrowse: true, canReadMetadata: true, canWatch: false, canPreview: false, canAddToLibrary: false, canRequestMaterialization: false },
} as unknown as LocationDescriptor;

function directory(name: string, path: string, id = name): BrowseEntry {
  return {
    ref: { kind: "ephemeral", browseSessionId: "tree-session", entryId: `entry-${id}` },
    pathRef: { id: `path-${id}` },
    name,
    displayPath: path,
    kind: "directory",
    materialization: "local",
  };
}

function page(entries: BrowseEntry[], requestId = "tree-request", nextCursor?: string): BrowsePage {
  return {
    sessionId: "tree-session",
    requestId,
    enumerationId: `enumeration-${requestId}`,
    entries,
    ...(nextCursor ? { nextCursor } : {}),
    completion: nextCursor ? "partial" : "complete",
    knownCount: entries.length,
  };
}

let root: Root;
let host: HTMLDivElement;
let container: HTMLDivElement;
const openBrowse = vi.fn(async () => ({ sessionId: "admitted-session" } as any));
const browseLocation = vi.fn(async () => ({ sessionId: "admitted-root-session" } as any));
const controller = { openBrowse, browseLocation } as unknown as FileLibraryExperienceController;
const navError = vi.fn();

async function flush() {
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

async function render(currentPath = "C:/", currentLocation = location) {
  await act(async () => {
    root.render(<DirectoryTree controller={controller} location={currentLocation} currentPath={currentPath} paneId="pane-tree" platform="win32" navigationError={null} onNavigationError={navError} t={t} />);
  });
  await flush();
}

function click(element: Element) {
  return act(async () => { (element as HTMLElement).click(); await Promise.resolve(); });
}

beforeEach(() => {
  (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
  window.localStorage.clear();
  document.body.innerHTML = "";
  host = document.createElement("div");
  document.body.appendChild(host);
  container = host;
  root = createRoot(host);
  openBrowse.mockClear();
  browseLocation.mockClear();
  navError.mockClear();
  vi.spyOn(fileWorkspaceApi, "locationBrowse").mockResolvedValue({ sessionId: "tree-session", rootPathRef: { id: "root" }, location } as any);
  vi.spyOn(fileWorkspaceApi, "browseStartEnumeration").mockResolvedValue(page([]));
  vi.spyOn(fileWorkspaceApi, "browseNextPage").mockResolvedValue(page([], "next"));
  vi.spyOn(fileWorkspaceApi, "browseReleasePage").mockResolvedValue(undefined);
  vi.spyOn(fileWorkspaceApi, "browseRetainPath").mockResolvedValue(undefined);
  vi.spyOn(fileWorkspaceApi, "browseReleasePath").mockResolvedValue(undefined);
  vi.spyOn(fileWorkspaceApi, "browseDispose").mockResolvedValue(undefined);
});

afterEach(async () => {
  await act(async () => root.unmount());
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("File Library directory tree", () => {
  it("loads child folders on demand and navigates through fresh Browse admission", async () => {
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockImplementation(async (request) =>
      request.pathRef.id === "root"
        ? page([directory("Folder A", "C:/Folder A", "a")], request.requestId)
        : page([directory("Nested", "C:/Folder A/Nested", "nested")], request.requestId),
    );
    await render();
    const tree = container.querySelector('[role="tree"]')!;
    const nodes = [...tree.querySelectorAll<HTMLElement>('[role="treeitem"]')];
    const folder = nodes.find((node) => node.querySelector(':scope > div > button[title="C:/Folder A"]'))!;
    expect(folder).toBeTruthy();
    expect(fileWorkspaceApi.browseStartEnumeration).toHaveBeenCalledWith(expect.objectContaining({ pathRef: { id: "root" }, pageSize: 32, query: expect.objectContaining({ entryKind: "directory" }) }));
    await click(folder.querySelector(':scope > div > button[aria-expanded="false"]')!);
    await flush();
    expect(folder.textContent).toContain("Nested");
    const folderNavigation = folder.querySelector(':scope > div > button[title="C:/Folder A"]')!;
    await click(folderNavigation);
    expect(openBrowse).toHaveBeenCalledWith({ platform: "windows", routingHint: "C:/Folder A", displayHint: "Folder A" });
  });

  it("keeps the internal Safe Trash namespace out of the user directory tree", async () => {
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockResolvedValue(page([
      directory(".zen-canvas-trash", "C:/.zen-canvas-trash", "safe-trash"),
      directory("Visible Folder", "C:/Visible Folder", "visible"),
      directory("operations", "C:/.zen-canvas-trash/operations", "safe-trash-operations"),
    ]));
    await render();
    expect(container.querySelector('button[title="C:/.zen-canvas-trash"]')).toBeNull();
    expect(container.querySelector('button[title="C:/.zen-canvas-trash/operations"]')).toBeNull();
    expect(container.querySelector('button[title="C:/Visible Folder"]')).toBeTruthy();
  });

  it("does not fetch later pages for a large folder until Load more is requested", async () => {
    const first = Array.from({ length: 32 }, (_, index) => directory(`Folder ${index}`, `C:/Folder ${index}`, `root-${index}`));
    const next = Array.from({ length: 8 }, (_, index) => directory(`Folder ${index + 32}`, `C:/Folder ${index + 32}`, `root-${index + 32}`));
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockResolvedValueOnce(page(first, "initial", "cursor-32"));
    vi.mocked(fileWorkspaceApi.browseNextPage).mockResolvedValueOnce(page(next, "last-page"));
    await render();
    expect(fileWorkspaceApi.browseStartEnumeration).toHaveBeenCalledTimes(1);
    expect(fileWorkspaceApi.browseNextPage).not.toHaveBeenCalled();
    const loadMore = [...container.querySelectorAll("button")].find((button) => button.textContent === t("fileLibraryNavigationTreeLoadMore"))!;
    expect(loadMore).toBeTruthy();
    await click(loadMore);
    await flush();
    expect(fileWorkspaceApi.browseNextPage).toHaveBeenCalledWith({ sessionId: "tree-session", cursor: "cursor-32", pageSize: 32 });
    expect(container.textContent).toContain("Folder 39");
  });

  it("advances one bounded page at a time even when pages contain no displayable path refs", async () => {
    const noPathRef = { ...directory("Hidden", "C:/Hidden"), pathRef: undefined } as unknown as BrowseEntry;
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockResolvedValueOnce(page([noPathRef], "initial", "cursor-a"));
    vi.mocked(fileWorkspaceApi.browseNextPage)
      .mockResolvedValueOnce(page([], "short-a", "cursor-b"))
      .mockResolvedValueOnce(page([], "short-b"));
    await render();
    const loadMore = () => [...container.querySelectorAll("button")].find((button) => button.textContent === t("fileLibraryNavigationTreeLoadMore"))!;
    expect(loadMore()).toBeTruthy();
    await click(loadMore());
    await flush();
    expect(fileWorkspaceApi.browseNextPage).toHaveBeenCalledTimes(1);
    expect(fileWorkspaceApi.browseNextPage).toHaveBeenLastCalledWith({ sessionId: "tree-session", cursor: "cursor-a", pageSize: 32 });
    expect(loadMore()).toBeTruthy();
    await click(loadMore());
    await flush();
    expect(fileWorkspaceApi.browseNextPage).toHaveBeenCalledTimes(2);
    expect(fileWorkspaceApi.browseNextPage).toHaveBeenLastCalledWith({ sessionId: "tree-session", cursor: "cursor-b", pageSize: 32 });
    expect(container.textContent).toContain(t("fileLibraryNavigationTreeEmpty"));
  });

  it("keeps the tree Browse session while only the active pane path changes", async () => {
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockResolvedValue(page([directory("Folder A", "C:/Folder A", "a")]));
    await render("C:/");
    expect(fileWorkspaceApi.locationBrowse).toHaveBeenCalledTimes(1);
    await render("C:/Folder A");
    expect(fileWorkspaceApi.locationBrowse).toHaveBeenCalledTimes(1);
    expect(container.querySelector('button[title="C:/Folder A"][aria-current="location"]')).toBeTruthy();
  });

  it("bounds automatic location scans across empty cursor pages", async () => {
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockResolvedValue(page([], "initial", "cursor-0"));
    let cursor = 0;
    vi.mocked(fileWorkspaceApi.browseNextPage).mockImplementation(async () => {
      cursor += 1;
      return page([], `empty-${cursor}`, `cursor-${cursor}`);
    });
    await render("C:/Missing");
    expect(fileWorkspaceApi.browseNextPage).toHaveBeenCalledTimes(64);
    expect(container.textContent).toContain(t("fileLibraryNavigationTreeScanLimit"));
  });

  it("locates and highlights the current descendant without restoring a live path reference", async () => {
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockImplementation(async (request) =>
      request.pathRef.id === "root"
        ? page([directory("Folder A", "C:/Folder A", "a")], request.requestId)
        : page([], request.requestId),
    );
    await render("C:/Folder A");
    await flush();
    const active = container.querySelector('button[title="C:/Folder A"][aria-current="location"]');
    expect(active).toBeTruthy();
    expect(fileWorkspaceApi.browseStartEnumeration).toHaveBeenCalledTimes(1);
    expect(openBrowse).not.toHaveBeenCalled();
  });

  it("supports arrow-key tree navigation and Enter activation", async () => {
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockResolvedValue(page([directory("Folder A", "C:/Folder A", "a")]));
    await render();
    const items = [...container.querySelectorAll<HTMLDivElement>('[role="treeitem"]')];
    expect(items).toHaveLength(2);
    items[0]!.focus();
    await act(async () => { items[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true })); });
    expect(document.activeElement).toBe(items[1]);
    await act(async () => { items[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
    expect(openBrowse).toHaveBeenCalledWith({ platform: "windows", routingHint: "C:/Folder A", displayHint: "Folder A" });
  });

  it("uses the admitted pane path when the current Browse Location is that directory", async () => {
    const nestedLocation = { ...location, displayName: "Folder After" } as LocationDescriptor;
    vi.mocked(fileWorkspaceApi.locationBrowse).mockResolvedValue({ sessionId: "tree-session", rootPathRef: { id: "root" }, location: nestedLocation } as any);
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockResolvedValue(page([]));
    await render("C:/QA/Folder After", nestedLocation);
    const activeRoot = container.querySelector('button[title="C:/QA/Folder After"][aria-current="location"]');
    expect(activeRoot).toBeTruthy();
    expect(container.textContent).not.toContain(t("fileLibraryNavigationTreeNotFound"));
  });

  it("rebases the tree root when navigation admits a new current folder", async () => {
    const nestedLocation = { ...location, displayName: "Folder After" } as LocationDescriptor;
    vi.mocked(fileWorkspaceApi.locationBrowse).mockResolvedValue({ sessionId: "tree-session-next", rootPathRef: { id: "root-next" }, location: nestedLocation } as any);
    vi.mocked(fileWorkspaceApi.browseStartEnumeration).mockResolvedValue(page([]));
    await render("C:/");
    await render("C:/QA/Folder After", nestedLocation);
    expect(container.querySelector('button[title="C:/QA/Folder After"][aria-current="location"]')).toBeTruthy();
    expect(container.textContent).not.toContain(t("fileLibraryNavigationTreeNotFound"));
  });

  it("persists a bounded, independent expansion list per pane", () => {
    const left = Array.from({ length: MAX_PERSISTED_TREE_PATHS + 8 }, (_, index) => `C:/Left/${index}`);
    const right = ["C:/Right/A"];
    expect(writeExpandedTreePaths("pane-left", left)).toBe(true);
    expect(writeExpandedTreePaths("pane-right", right)).toBe(true);
    expect(readExpandedTreePaths("pane-left")).toHaveLength(MAX_PERSISTED_TREE_PATHS);
    expect(readExpandedTreePaths("pane-right")).toEqual(right);
  });
});
