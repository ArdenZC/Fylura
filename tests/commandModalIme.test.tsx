// @vitest-environment happy-dom
import { act, type RefObject } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CommandModal } from "../src/components/CommandModal";
import { searchRuntimeApi as tauriApi } from "../src/api/searchRuntimeApi";
import type { GlobalSearchRequest, GlobalSearchResponse } from "../src/types/domain";
import { globalSearchFeedback } from "../src/components/spotlight/globalSearchFeedback";
import { makeTranslator } from "../src/i18n";

const t = makeTranslator("en");

function setInputValue(input: HTMLInputElement, value: string, composing = false) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
  const event = new InputEvent("input", { bubbles: true, inputType: "insertText", data: value });
  Object.defineProperty(event, "isComposing", { configurable: true, value: composing });
  Object.defineProperty(event, "keyCode", { configurable: true, value: composing ? 229 : 0 });
  input.dispatchEvent(event);
}

function scopeResponse(request: GlobalSearchRequest, sourceStatus: string): GlobalSearchResponse {
  const ready = sourceStatus === "ready";
  return {
    version: 2, requestId: request.requestId, normalizedQuery: request.query,
    results: [], sourceRevision: `scope-${sourceStatus}`,
    resultState: ready ? "empty" : "pending", collectionComplete: ready,
    sourceHealth: [{ sourceId: "fixture-source", enabled: true, provider: "windows_mft_usn", status: sourceStatus, lastError: null, updatedAt: 1 }],
    indexStatus: { platform: "windows", enabled: true, status: ready ? "ready" : "partial",
      processedEntries: ready ? 1 : 0, collectionComplete: ready, totalEntries: ready ? 1 : 0,
      indexedVolumes: 1, readyVolumes: ready ? 1 : 0, pendingVolumes: ready ? 0 : 1, lastSyncAt: null, lastError: null }
  };
}

it("distinguishes source readiness without declaring unindexed snapshots empty", () => {
  const request = { version: 2 as const, requestId: "readiness", query: "fixture", limit: 20, offset: 0 };
  for (const [status, title] of [
    ["discovered", "globalSearchNotIndexedTitle"], ["indexing", "globalIndexStatusIndexing"],
    ["paused", "globalIndexStatusPaused"], ["rebuild_required", "globalIndexStatusRebuildRequired"],
    ["unavailable", "globalIndexStatusUnavailable"],
    ["permission_required", "globalIndexStatusPermissionRequired"],
    ["error", "globalIndexStatusError"]
  ] as const) {
    const response = scopeResponse(request, status);
    if (status === "permission_required" || status === "error" || status === "unavailable") {
      response.indexStatus.status = status;
    }
    const feedback = globalSearchFeedback(response, t);
    expect(feedback.title).toBe(t(title));
    expect(feedback.title).not.toBe(t("commandNoResultsTitle"));
  }
  expect(globalSearchFeedback(scopeResponse(request, "ready"), t).title).toBe(t("commandNoResultsTitle"));
  for (const [status, title] of [
    ["database_busy", "globalIndexStatusDatabaseBusy"],
    ["service_busy", "globalIndexStatusServiceBusy"]
  ] as const) {
    const busy = scopeResponse(request, status);
    busy.indexStatus.status = status;
    expect(globalSearchFeedback(busy, t).title).toBe(t(title));
  }
  const unverified = scopeResponse(request, "ready");
  unverified.collectionComplete = false;
  unverified.resultState = "pending";
  expect(globalSearchFeedback(unverified, t).description).toBe(t("globalSearchUnverifiedDesc"));
});

describe("mounted CommandModal IME behavior", () => {
  let root: Root | null = null;
  let container: HTMLDivElement;

  beforeEach(() => {
    vi.useFakeTimers();
    // Happy DOM 21 eagerly creates Animation.finished. Cancellation during
    // unmount rejects it even when Motion never observes that native promise.
    // Handle only the expected abort on animations in this mounted fixture;
    // unexpected animation errors and unrelated unhandled rejections still fail.
    const nativeAnimate = Element.prototype.animate;
    vi.spyOn(Element.prototype, "animate").mockImplementation(function (this: Element, keyframes, options) {
      const animation = nativeAnimate.call(this, keyframes, options);
      void animation.finished.catch((error: unknown) => {
        if (!(error instanceof DOMException && error.name === "AbortError")) throw error;
      });
      return animation;
    });
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    act(() => root?.unmount());
    root = null;
    container.remove();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("does not query or execute commands during composition, then commits one final value", async () => {
    const searchGlobalEntries = vi.spyOn(tauriApi, "searchGlobalEntries").mockResolvedValue({
      version: 2,
      requestId: "ime-request",
      normalizedQuery: "中",
      results: [],
      indexStatus: {
        platform: "browser",
        enabled: true,
        status: "ready",
        processedEntries: 1,
        collectionComplete: true,
        totalEntries: 1,
        indexedVolumes: 1,
        readyVolumes: 1,
        pendingVolumes: 0,
        lastSyncAt: null,
        lastError: null
      },
      collectionComplete: true,
      resultState: "empty",
      sourceRevision: "ime-source",
      sourceHealth: []
    });
    vi.spyOn(tauriApi, "onSearchWindowState").mockResolvedValue(() => undefined);
    vi.spyOn(tauriApi, "getSearchWindowState").mockResolvedValue({ sessionId: 1, revision: 1, phase: "visible_collapsed" });
    const openGlobalSearchResult = vi.spyOn(tauriApi, "openGlobalSearchResult").mockResolvedValue();
    const revealGlobalSearchResult = vi.spyOn(tauriApi, "revealGlobalSearchResult").mockResolvedValue();
    const activateSearchResult = vi.spyOn(tauriApi, "activateSearchResult").mockResolvedValue();
    const setView = vi.fn();
    const setSelectedFileId = vi.fn();
    const onClose = vi.fn();
    const inputRef = { current: null } as RefObject<HTMLInputElement | null>;

    await act(async () => {
      root = createRoot(container);
      root.render(
        <CommandModal
          inputRef={inputRef}
          setView={setView}
          setSelectedFileId={setSelectedFileId}
          onClose={onClose}
          platform="browser"
          t={t}
          standalone
        />
      );
    });
    const input = container.querySelector<HTMLInputElement>('input[role="combobox"]')!;
    expect(input).toBeTruthy();

    await act(async () => input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    for (const value of ["z", "zh", "zhong"]) {
      await act(async () => setInputValue(input, value, true));
    }
    await act(async () => {
      vi.advanceTimersByTime(1_000);
    });
    expect(searchGlobalEntries).not.toHaveBeenCalled();

    for (const key of ["Enter", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"]) {
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })));
    }
    expect(input.getAttribute("aria-activedescendant")).toBeNull();
    expect(openGlobalSearchResult).not.toHaveBeenCalled();
    expect(revealGlobalSearchResult).not.toHaveBeenCalled();
    expect(activateSearchResult).not.toHaveBeenCalled();
    expect(setView).not.toHaveBeenCalled();
    expect(setSelectedFileId).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => {
      setInputValue(input, "中");
      input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
    });
    await act(async () => {
      vi.advanceTimersByTime(60);
      await Promise.resolve();
    });

    expect(searchGlobalEntries).toHaveBeenCalledTimes(1);
    expect(searchGlobalEntries.mock.calls[0][0].query).toBe("中");
  });
  it("shows unindexed coverage, refreshes the same query after readiness, and activates only fixture IDs", async () => {
    let sourceStatus = "discovered";
    const query = vi.spyOn(tauriApi, "searchGlobalEntries").mockImplementation(async (request) => {
      const response = scopeResponse(request, sourceStatus);
      if (sourceStatus === "ready") {
        response.resultState = "complete";
        response.results = [{
          id: "owned-fixture-id", volumeId: "fixture-source", platformFileId: "fixture-file", name: "m1-fixture-中文.txt",
          path: "F:/owned-fixture/m1-fixture-中文.txt", extension: "txt", isDirectory: false, size: 1,
          createdAtFs: 1, modifiedAtFs: 1, fileAttributes: 0, isHidden: false, isSystem: false,
          sourceProvider: "windows_mft_usn", managed: false, rank: 0
        }];
      }
      return response;
    });
    vi.spyOn(tauriApi, "onSearchWindowState").mockResolvedValue(() => undefined);
    vi.spyOn(tauriApi, "getSearchWindowState").mockResolvedValue({ sessionId: 1, revision: 1, phase: "visible_collapsed" });
    vi.spyOn(tauriApi, "resizeSearchWindow").mockResolvedValue({ sessionId: 1, revision: 2, phase: "visible_expanded" });
    vi.spyOn(tauriApi, "hideSearchWindow").mockResolvedValue({ sessionId: 1, revision: 3, phase: "hidden" });
    const open = vi.spyOn(tauriApi, "openGlobalSearchResult").mockResolvedValue();
    const reveal = vi.spyOn(tauriApi, "revealGlobalSearchResult").mockResolvedValue();
    const activation = vi.spyOn(tauriApi, "activateSearchResult").mockRejectedValue(new Error("search_navigation_rejected"));
    const inputRef = { current: null } as RefObject<HTMLInputElement | null>;
    await act(async () => {
      root = createRoot(container);
      root.render(<CommandModal inputRef={inputRef} setView={vi.fn()} setSelectedFileId={vi.fn()} onClose={vi.fn()} platform="win32" t={t} standalone />);
    });
    const input = container.querySelector<HTMLInputElement>('input[role="combobox"]')!;
    await act(async () => { setInputValue(input, "m1-fixture"); });
    await act(async () => { await vi.advanceTimersByTimeAsync(100); });
    expect(container.textContent).toContain(t("globalSearchNotIndexedTitle"));
    expect(container.textContent).toContain(t("globalSearchNotIndexedDesc"));
    expect(container.textContent).not.toContain(t("commandNoResultsTitle"));
    expect(query).toHaveBeenCalledOnce();
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(query).toHaveBeenCalledOnce();
    sourceStatus = "ready";
    const refresh = [...container.querySelectorAll("button")].find((button) => button.textContent === t("globalSearchRefreshResults"));
    expect(refresh).toBeTruthy();
    await act(async () => { refresh!.click(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(200); });
    await act(async () => { await vi.advanceTimersByTimeAsync(100); });
    expect(container.textContent).toContain("m1-fixture-中文.txt");
    expect(container.textContent).not.toContain(t("globalSearchNotIndexedTitle"));
    expect(query.mock.calls.every(([request]) => request.query === "m1-fixture")).toBe(true);
    expect(query.mock.calls.length).toBeLessThanOrEqual(3);
    const tab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    await act(async () => { input.dispatchEvent(tab); });
    expect(tab.defaultPrevented).toBe(false); // At the last result, Tab can reach controls.
    const readyRefresh = [...container.querySelectorAll("button")].find((button) => button.textContent === t("globalSearchRefreshResults"))!;
    const refreshEnter = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    await act(async () => { readyRefresh.focus(); readyRefresh.dispatchEvent(refreshEnter); });
    expect(refreshEnter.defaultPrevented).toBe(false); // Native button activation, not file navigation.
    expect(open).not.toHaveBeenCalled();
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true })); });
    expect(reveal).toHaveBeenCalledWith("owned-fixture-id");
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(open).toHaveBeenCalledWith("owned-fixture-id");
    const libraryAction = [...container.querySelectorAll("button")].find((button) => button.textContent?.includes(t("globalSearchShowInLibrary")))!;
    expect(libraryAction).toBeTruthy();
    await act(async () => { libraryAction.click(); });
    expect(activation).toHaveBeenCalledWith("library", null, expect.objectContaining({ sessionId: 1 }), null, "owned-fixture-id");
    expect(container.textContent).toContain(t("globalSearchLibraryFailed"));
    expect(input.readOnly).toBe(false);
    expect(Object.keys(tauriApi)).not.toContain("rebuildGlobalIndexSource");
    expect(Object.keys(tauriApi)).not.toContain("setGlobalIndexSourceEnabled");
  });

});
