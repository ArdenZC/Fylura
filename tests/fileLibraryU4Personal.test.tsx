// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, it, expect, vi } from "vitest";
import { mockInvokeCommand } from "../src/api/browserMockApi";
import { libraryApi } from "../src/api/libraryApi";
import { makeTranslator } from "../src/i18n";
import type { FilePersonalMetadataResponse, FilePersonalMetadataRequest } from "../src/types/domain";
import { FilePersonalMetadataEditor } from "../src/views/fileLibrary/context/FilePersonalMetadata";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const request = (request: FilePersonalMetadataRequest) => mockInvokeCommand<FilePersonalMetadataResponse>("file_library_personal_metadata", { request });

describe("U4 user metadata lifecycle and owner isolation", () => {
  it("mock persists notes/favorites, returns IDs, rejects stale edits and leaves other files untouched", async () => {
    localStorage.removeItem("zc-u4-mock-personal");
    const original = await request({ kind: "get", fileId: "mock-report" });
    expect(original.metadata?.note).toBe("");
    await request({ kind: "update", fileId: "mock-report", note: "owner note", favorite: true, expectedRevision: 0 });
    expect((await request({ kind: "get", fileId: "mock-report" })).metadata).toMatchObject({ note: "owner note", favorite: true, revision: 1 });
    expect((await request({ kind: "get", fileId: "mock-archive" })).metadata?.note).toBe("");
    expect((await request({ kind: "list", favorites: true, offset: 0 })).items.map((item) => item.fileId)).toEqual(["mock-report"]);
    await expect(request({ kind: "update", fileId: "mock-report", note: "stale", favorite: false, expectedRevision: 0 })).rejects.toThrow();
    await request({ kind: "update", fileId: "mock-report", note: "", favorite: false, expectedRevision: 1 });
    expect((await request({ kind: "list", favorites: false, offset: 0 })).items).toEqual([]);
    expect((await request({ kind: "get", fileId: "mock-report" })).metadata?.revision).toBe(2);
    await expect(request({ kind: "get", fileId: "ephemeral-browse-entry" })).rejects.toThrow();
  });

  it("ignores a late read for A after selection changes to B", async () => {
    let resolveA!: (value: FilePersonalMetadataResponse) => void;
    const pending = new Promise<FilePersonalMetadataResponse>((resolve) => { resolveA = resolve; });
    const spy = vi.spyOn(libraryApi, "personalMetadata").mockImplementation((value) => value.kind === "get" && value.fileId === "A" ? pending : Promise.resolve({ metadata: { fileId: "B", note: "B note", favorite: false, revision: 1, editable: true }, items: [], hasMore: false }));
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    const render = (id: string) => root.render(createElement(FilePersonalMetadataEditor, { key: id, fileId: id, t: makeTranslator("en") }));
    try {
      await act(async () => { render("A"); });
      await act(async () => { render("B"); });
      await act(async () => { resolveA({ metadata: { fileId: "A", note: "private A note", favorite: true, revision: 2, editable: true }, items: [], hasMore: false }); });
      expect(container.querySelector("textarea")?.value).toBe("B note");
      expect(container.textContent).not.toContain("private A note");
    } finally { await act(async () => root.unmount()); container.remove(); spy.mockRestore(); }
  });

  it("shows conflicts explicitly and prevents another blind write", async () => {
    const spy = vi.spyOn(libraryApi, "personalMetadata").mockImplementation((value) => value.kind === "get" ? Promise.resolve({ metadata: { fileId: "A", note: "old", favorite: false, revision: 1, editable: true }, items: [], hasMore: false }) : Promise.reject(new Error("conflict")));
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    try {
      await act(async () => root.render(createElement(FilePersonalMetadataEditor, { fileId: "A", t: makeTranslator("en") })));
      const favorite = [...container.querySelectorAll("button")].find((button) => button.textContent === "Favorite file")!;
      await act(async () => favorite.click());
      expect(container.querySelector('[role="alert"]')?.textContent).toContain("another pane edited");
      expect(favorite.disabled).toBe(true);
      expect(container.querySelector("textarea")?.value).toBe("old");
    } finally { await act(async () => root.unmount()); container.remove(); spy.mockRestore(); }
  });
});

import { useSelectedPreview } from "../src/views/fileLibrary/preview/useSelectedPreview";
import type { PreviewSourceProjection } from "../src/views/fileLibrary/preview/previewSource";
import type { PreviewSiblingNavigationProjection } from "../src/views/fileLibrary/preview/previewSiblingNavigation";

it("selected preview navigation preserves owner state and rejects an expired neighborhood", async () => {
  let navigation: PreviewSiblingNavigationProjection | null = null;
  const controller = { open: vi.fn(() => true), observeSource: vi.fn(), setSiblingNavigation: vi.fn((value: PreviewSiblingNavigationProjection | null) => { navigation = value; }) };
  let open!: (index: number, trigger: HTMLElement) => boolean;
  const sources: PreviewSourceProjection[] = ["A", "B"].map((id) => ({ key: `library:g:${id}`, generation: "g", source: "library", previewSource: { kind: "managed", fileId: id }, displayName: id, entryKind: "file" }));
  function Harness({ sources }: { sources: PreviewSourceProjection[] }) { open = useSelectedPreview(controller, sources); return null; }
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  try {
    await act(async () => root.render(createElement(Harness, { sources })));
    expect(open(0, container)).toBe(true);
    const first = navigation! as PreviewSiblingNavigationProjection;
    expect(first.loadedCount).toBe(2);
    expect(first.move("next")).toBe(true);
    expect(controller.observeSource).toHaveBeenLastCalledWith(sources[1]);
    await act(async () => root.render(createElement(Harness, { sources: [] })));
    expect(first.move("next")).toBe(false);
    expect(navigation).toBeNull();
  } finally { await act(async () => root.unmount()); container.remove(); }
});

it("Browse Reveal resolves live refs in the mock and honestly refuses native GUI claims", async () => {
  const { mockFileWorkspaceInvoke } = await import("../src/api/fileWorkspaceMockApi");
  const opened = await mockFileWorkspaceInvoke<import("../src/types/fileWorkspace").BrowseOpenResponse>("file_workspace_browse_open", { request: { platform: "windows", routingHint: "C:/U4/Reveal", displayHint: "Reveal test" } });
  const page = await mockFileWorkspaceInvoke<import("../src/types/fileWorkspace").BrowsePage>("file_workspace_browse_start_enumeration", { request: { sessionId: opened.sessionId, requestId: "u4-reveal", pathRef: opened.rootPathRef, pageSize: 16 } });
  const entryRef = page.entries[0].ref;
  await expect(mockFileWorkspaceInvoke("file_workspace_browse_reveal", { request: { entryRef } })).rejects.toThrow("browser_mock_reveal_unavailable");
  await expect(mockFileWorkspaceInvoke("file_workspace_browse_reveal", { request: { entryRef: { ...entryRef, entryId: "foreign-ref" } } })).rejects.toThrow();
  await mockFileWorkspaceInvoke("file_workspace_browse_dispose", { request: { sessionId: opened.sessionId } });
  await expect(mockFileWorkspaceInvoke("file_workspace_browse_reveal", { request: { entryRef } })).rejects.toThrow();
});

it("managed Inspector keeps unknown timestamp sentinels explicit", async () => {
  const { renderToStaticMarkup } = await import("react-dom/server");
  const { FileLibraryInspector } = await import("../src/views/vault/components/FileLibraryInspector");
  const detail = await mockInvokeCommand<import("../src/types/domain").FileLibraryDetail>("get_file_library_detail", { fileId: "mock-report" });
  const html = renderToStaticMarkup(createElement(FileLibraryInspector, { selectedIds: new Set([detail.id]), selectedFiles: [], selectionKind: "explicit", selectedCount: 1, detail: { ...detail, createdAt: 0, modifiedAt: 0, nativeSemantics: null }, selectionSummary: null, isLoading: false, error: null, language: "en", t: makeTranslator("en"), onPreview: vi.fn(), onReveal: vi.fn(), onViewSuggestions: vi.fn(), onViewOperations: vi.fn(), onOpenContentUnderstanding: vi.fn(), onClearSelection: vi.fn(), onRetryDetail: vi.fn() }));
  const container = document.createElement("div"); container.innerHTML = html;
  for (const label of ["Created", "Modified", "Current status"]) {
    const field = [...container.querySelectorAll("dt")].find((item) => item.textContent === label)!;
    expect(field.nextElementSibling?.textContent).toBe("Unknown");
  }
});
