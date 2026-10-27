import { describe, expect, it } from "vitest";
import {
  WorkspaceSession,
  navigationTargetFromLibraryRestoreLocator,
  parseWorkspaceRestoreMetadata
} from "../src/fileWorkspace";
import type { NavigationTarget, WorkspaceRestoreLocator } from "../src/types/fileWorkspace";

const libraryOne: NavigationTarget = {
  kind: "library",
  source: "saved_view",
  key: "recent-files"
};
const libraryTwo: NavigationTarget = {
  kind: "library",
  source: "search",
  key: "quarterly-report"
};
const libraryThree: NavigationTarget = {
  kind: "library",
  source: "tag",
  key: "important"
};
const browseDocuments: NavigationTarget = {
  kind: "browse",
  location: {
    kind: "ephemeral",
    browseSessionId: "browse-session-1",
    locationId: "location-documents"
  },
  pathRef: { id: "path-documents" }
};

const browseRestoreLocator: WorkspaceRestoreLocator = {
  kind: "browse",
  platform: "windows",
  routingHint: "Documents",
  displayHint: "Documents"
};

describe("WorkspaceSession navigation core", () => {
  it("clones and restores pane-local sort while carrying it to new Browse folders", () => {
    const session = new WorkspaceSession({ initialTarget: browseDocuments });
    const sort = { field: "size" as const, direction: "desc" as const, foldersFirst: true };
    expect(session.setPresentation({ browseQuery: { text: "filter", entryKind: "file", sort } })).toBe(true);
    const peer = new WorkspaceSession({ initialTarget: browseDocuments });
    expect(peer.getState().presentation.browseQuery).toBeUndefined();
    const newFolder = { ...browseDocuments, pathRef: { id: "another-folder" } };
    expect(session.navigate(newFolder)).toBe(true);
    expect(session.getState().presentation.browseQuery).toEqual({ text: null, entryKind: "all", sort });
    session.back();
    expect(session.getState().presentation.browseQuery?.text).toBe("filter");
    sort.direction = "asc" as "desc";
    expect(session.getState().presentation.browseQuery?.sort?.direction).toBe("desc");
    expect(session.setPresentation({ browseQuery: { text: null, entryKind: "all", sort: { field: "path", direction: "asc", foldersFirst: true } } } as never)).toBe(false);
  });

  it("keeps Library and Browse targets in one chronological Back/Forward history", () => {
    const session = new WorkspaceSession();

    expect(session.navigate(libraryOne)).toBe(true);
    expect(session.navigate(browseDocuments, { restoreLocator: browseRestoreLocator })).toBe(true);
    expect(session.navigate(libraryTwo)).toBe(true);
    expect(session.getState().history).toEqual([libraryOne, browseDocuments, libraryTwo]);

    expect(session.back()).toBe(true);
    expect(session.currentTarget).toEqual(browseDocuments);
    expect(session.back()).toBe(true);
    expect(session.currentTarget).toEqual(libraryOne);
    expect(session.forward()).toBe(true);
    expect(session.currentTarget).toEqual(browseDocuments);
    expect(session.forward()).toBe(true);
    expect(session.currentTarget).toEqual(libraryTwo);
  });

  it("records direct mode switches as chronological steps while retaining last targets", () => {
    const session = new WorkspaceSession({ initialTarget: libraryOne });
    session.navigate(browseDocuments, { restoreLocator: browseRestoreLocator });
    session.navigate(libraryTwo);
    const historyBeforeSwitch = session.getState().history;

    expect(session.switchToBrowse()).toBe(true);
    expect(session.currentTarget).toEqual(browseDocuments);
    expect(session.getState().history).toEqual([...historyBeforeSwitch, browseDocuments]);
    expect(session.getState().historyIndex).toBe(3);
    expect(session.getState().lastLibraryTarget).toEqual(libraryTwo);
    expect(session.getState().lastBrowseTarget).toEqual(browseDocuments);
    expect(session.back()).toBe(true);
    expect(session.currentTarget).toEqual(libraryTwo);
    expect(session.forward()).toBe(true);
    expect(session.currentTarget).toEqual(browseDocuments);

    expect(session.switchToLibrary()).toBe(true);
    expect(session.currentTarget).toEqual(libraryTwo);
    expect(session.getState().history).toEqual([...historyBeforeSwitch, browseDocuments, libraryTwo]);
    expect(session.getState().historyIndex).toBe(4);
    expect(session.back()).toBe(true);
    expect(session.currentTarget).toEqual(browseDocuments);
  });

  it("truncates only forward history after navigating from an older position", () => {
    const session = new WorkspaceSession({ initialTarget: libraryOne });
    session.navigate(browseDocuments, { restoreLocator: browseRestoreLocator });
    session.navigate(libraryTwo);
    session.back();
    session.back();

    expect(session.currentTarget).toEqual(libraryOne);
    expect(session.navigate(libraryThree)).toBe(true);
    expect(session.getState().history).toEqual([libraryOne, libraryThree]);
    expect(session.forward()).toBe(false);
    expect(session.getState().lastBrowseTarget).toBeNull();
  });

  it("revokes stale request publication after navigation and advances the epoch", () => {
    const session = new WorkspaceSession({ initialTarget: libraryOne });
    const request = session.beginRequest();
    const epochAtRequest = request.epoch;

    expect(session.canPublish(request)).toBe(true);
    expect(session.navigate(browseDocuments)).toBe(true);
    expect(session.requestEpoch).toBeGreaterThan(epochAtRequest);
    expect(session.canPublish(request)).toBe(false);

    const currentRequest = session.beginRequest();
    expect(session.canPublish(currentRequest)).toBe(true);
    expect(session.isEpochCurrent(currentRequest.epoch)).toBe(true);
  });

  it("keeps sibling request tokens current until one navigation invalidates both", () => {
    const session = new WorkspaceSession({ initialTarget: libraryOne });
    const firstRequest = session.beginRequest();
    const secondRequest = session.beginRequest();

    expect(secondRequest.epoch).toBe(firstRequest.epoch);
    expect(session.canPublish(firstRequest)).toBe(true);
    expect(session.canPublish(secondRequest)).toBe(true);

    expect(session.navigate(browseDocuments)).toBe(true);
    expect(session.requestEpoch).toBe(firstRequest.epoch + 1);
    expect(session.canPublish(firstRequest)).toBe(false);
    expect(session.canPublish(secondRequest)).toBe(false);
  });

  it("restores presentation for each history entry across Back/Forward and mode switching", () => {
    const libraryOnePresentation = { viewMode: "grid" as const, scrollAnchor: "library-one" };
    const browsePresentation = { viewMode: "list" as const, scrollAnchor: "browse-documents" };
    const libraryTwoPresentation = { viewMode: "grid" as const, scrollAnchor: "library-two" };
    const session = new WorkspaceSession({
      initialTarget: libraryOne,
      presentation: libraryOnePresentation
    });

    expect(session.setPresentation({ viewMode: "grid", scrollAnchor: "library-one-updated" })).toBe(true);
    const updatedLibraryOnePresentation = { viewMode: "grid" as const, scrollAnchor: "library-one-updated" };
    expect(session.navigate(browseDocuments, {
      restoreLocator: browseRestoreLocator,
      presentation: browsePresentation
    })).toBe(true);
    expect(session.navigate(libraryTwo, { presentation: libraryTwoPresentation })).toBe(true);
    expect(session.getState().presentation).toEqual(libraryTwoPresentation);

    expect(session.back()).toBe(true);
    expect(session.getState().presentation).toEqual(browsePresentation);
    expect(session.back()).toBe(true);
    expect(session.getState().presentation).toEqual(updatedLibraryOnePresentation);
    expect(session.forward()).toBe(true);
    expect(session.getState().presentation).toEqual(browsePresentation);
    expect(session.forward()).toBe(true);

    expect(session.switchToBrowse()).toBe(true);
    expect(session.currentTarget).toEqual(browseDocuments);
    expect(session.getState().presentation).toEqual(browsePresentation);
    expect(session.back()).toBe(true);
    expect(session.currentTarget).toEqual(libraryTwo);
    expect(session.getState().presentation).toEqual(libraryTwoPresentation);
  });

  it("makes disposal deterministic and revokes every outstanding publication right", () => {
    const session = new WorkspaceSession({ initialTarget: libraryOne });
    const request = session.beginRequest();
    const epochBeforeDispose = session.requestEpoch;

    expect(session.dispose()).toBe(true);
    expect(session.disposed).toBe(true);
    expect(session.requestEpoch).toBeGreaterThan(epochBeforeDispose);
    expect(session.canPublish(request)).toBe(false);
    expect(session.navigate(libraryTwo)).toBe(false);
    expect(session.back()).toBe(false);
    expect(session.forward()).toBe(false);
    expect(session.switchMode("browse")).toBe(false);
    expect(session.serializeRestoreMetadata()).toBeNull();
    expect(session.dispose()).toBe(false);
  });

  it("serializes only safe restore metadata and preserves Library source plus key", () => {
    const session = new WorkspaceSession({
      initialTarget: libraryOne,
      presentation: { viewMode: "grid", scrollAnchor: "entry-key-1" }
    });

    const serialized = session.serializeRestoreMetadata();
    expect(serialized).toEqual({
      version: 1,
      locator: {
        kind: "library",
        source: "saved_view",
        key: "recent-files"
      },
      presentation: { viewMode: "grid", scrollAnchor: "entry-key-1" }
    });
    expect(JSON.stringify(serialized)).not.toContain("browseSessionId");
    expect(JSON.stringify(serialized)).not.toContain("pathRef");
    expect(navigationTargetFromLibraryRestoreLocator(serialized!.locator)).toEqual(libraryOne);
  });

  it("serializes Browse routing hints without reviving ephemeral session/path refs", () => {
    const session = new WorkspaceSession();
    session.navigate(browseDocuments, { restoreLocator: browseRestoreLocator });

    const serialized = session.serializeRestoreMetadata();
    expect(serialized?.locator).toEqual(browseRestoreLocator);
    expect(JSON.stringify(serialized)).not.toContain("browseSessionId");
    expect(JSON.stringify(serialized)).not.toContain("locationId");
    expect(JSON.stringify(serialized)).not.toContain("pathRef");

    const parsed = parseWorkspaceRestoreMetadata(serialized);
    expect(parsed).toEqual({ ok: true, metadata: serialized });
  });

  it("fails closed on invalid restore data instead of guessing a target", () => {
    const invalid = parseWorkspaceRestoreMetadata({
      version: 1,
      locator: {
        kind: "browse",
        platform: "windows",
        routingHint: "Documents",
        pathRef: { id: "path-from-old-process" }
      }
    });
    expect(invalid).toEqual({ ok: false, reason: "invalid_locator" });

    const unknownVersion = parseWorkspaceRestoreMetadata({
      version: 2,
      locator: { kind: "library", source: "saved_view", key: "recent-files" }
    });
    expect(unknownVersion).toEqual({ ok: false, reason: "unsupported_version" });
  });
});
