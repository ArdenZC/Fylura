import { describe, expect, it, vi } from "vitest";
import { fileWorkspaceApi, type FileWorkspaceApi } from "../src/api/fileWorkspaceApi";
import { withOneOffOrganizeSource } from "../src/views/organize/oneOffOrganizeSource";
import type { BrowseEntry, BrowseOpenResponse, BrowsePage } from "../src/types/fileWorkspace";

const request = { platform: "windows" as const, routingHint: "C:/Users/test/Downloads" };
const folder: BrowseOpenResponse = {
  sessionId: "one-off",
  rootPathRef: { id: "folder" },
  location: {
    ref: { kind: "ephemeral", browseSessionId: "one-off", locationId: "downloads" },
    displayName: "Downloads", kind: "unknown", availability: "available", freshness: "not_applicable",
    capabilities: { canBrowse: true, canReadMetadata: true, canPreview: true, canWatch: false, canRequestMaterialization: false, canAddToLibrary: false }
  }
};
const entry = (id: string): BrowseEntry => ({
  ref: { kind: "ephemeral", browseSessionId: "one-off", entryId: id },
  name: id, displayPath: `presentation-only/${id}`, kind: "file", materialization: "local"
});
const page = (entries: BrowseEntry[], extra: Partial<BrowsePage> = {}): BrowsePage => ({
  sessionId: "one-off", requestId: "read", enumerationId: "enumeration", entries, completion: "complete", ...extra
});
function fixture(pages: BrowsePage[]) {
  const api: FileWorkspaceApi = {
    ...fileWorkspaceApi,
    browseOpen: vi.fn(async () => folder),
    browseStartEnumeration: vi.fn(async () => pages.shift()!),
    browseNextPage: vi.fn(async () => pages.shift()!),
    browseCancel: vi.fn(async () => undefined),
    browseReleasePage: vi.fn(async () => undefined),
    browseReleasePath: vi.fn(async () => undefined),
    browseDispose: vi.fn(async () => undefined)
  };
  return api;
}

describe("one-off Organize folder source", () => {
  it("keeps every page's opaque refs alive through Plan capture, then releases the source", async () => {
    const entries = [entry("invoice.pdf"), entry("photo.jpg")];
    const api = fixture([page(entries.slice(0, 1), { nextCursor: "next", completion: "partial" }), page(entries.slice(1))]);
    const consume = vi.fn(async (source) => {
      expect(source.entries).toEqual(entries);
      expect(source.folder.rootPathRef).toEqual({ id: "folder" });
      expect(api.browseReleasePage).not.toHaveBeenCalled();
      expect(api.browseDispose).not.toHaveBeenCalled();
      return "captured-plan";
    });
    await expect(withOneOffOrganizeSource(request, consume, { api, maxItems: 10 })).resolves.toBe("captured-plan");
    expect(api.browseOpen).toHaveBeenCalledWith(request);
    expect(api.browseStartEnumeration).toHaveBeenCalledWith(expect.objectContaining({ query: { text: null, entryKind: "all" } }));
    expect(api.browseReleasePage).toHaveBeenCalledTimes(2);
    expect(api.browseDispose).toHaveBeenCalledWith({ sessionId: "one-off" });
  });

  it.each([
    [page([entry("readable")], { skippedEntries: 1 }), "one_off_folder_incomplete"],
    [page([], { completion: "partial" }), "one_off_folder_incomplete"],
    [page([entry("one"), entry("two")]), "one_off_folder_too_large"]
  ])("does not turn incomplete coverage or a truncated selection into a Plan", async (result, error) => {
    const api = fixture([result]);
    const consume = vi.fn();
    await expect(withOneOffOrganizeSource(request, consume, { api, maxItems: 1 })).rejects.toThrow(error);
    expect(consume).not.toHaveBeenCalled();
    expect(api.browseDispose).toHaveBeenCalledOnce();
  });

  it("cancels between pages without capturing a partial Plan", async () => {
    const signal = new AbortController();
    const api = fixture([]);
    api.browseStartEnumeration = vi.fn(async () => {
      signal.abort();
      return page([entry("one")], { nextCursor: "next", completion: "partial" });
    });
    const consume = vi.fn();
    await expect(withOneOffOrganizeSource(request, consume, { api, maxItems: 10, signal: signal.signal })).rejects.toMatchObject({ name: "AbortError" });
    expect(api.browseNextPage).not.toHaveBeenCalled();
    expect(consume).not.toHaveBeenCalled();
    expect(api.browseDispose).toHaveBeenCalledOnce();
  });

  it("releases pages and admission after backend Plan capture rejects", async () => {
    const api = fixture([page([entry("one")])]);
    await expect(withOneOffOrganizeSource(request, async () => { throw new Error("capture failed"); }, { api, maxItems: 10 })).rejects.toThrow("capture failed");
    expect(api.browseReleasePage).toHaveBeenCalledOnce();
    expect(api.browseDispose).toHaveBeenCalledOnce();
  });
});
