// @vitest-environment happy-dom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import {
  FileWorkspaceController,
  WorkspaceSession,
  parseWorkspaceRestoreMetadata
} from "../src/fileWorkspace";
import { makeTranslator } from "../src/i18n";
import { explicitSingleSelectionId } from "../src/store/useFileLibraryV2Store";
import { libraryInspectorContentKind, type FileLibraryInspectorProps } from "../src/views/vault/components/FileLibraryInspector";
import { WorkspaceCommandBar } from "../src/views/fileLibrary/FileLibraryWorkspace";
import { FileLibraryExperienceController } from "../src/views/fileLibrary/fileLibraryExperience";
import {
  browseSizeProjection,
  createBrowseContextProjection,
  createLibraryContextProjection,
  libraryContextContentKind,
  libraryContextSelectionCount
} from "../src/views/fileLibrary/context/contextPanelProjection";
import type { BrowsePresentationEntry } from "../src/views/fileLibrary/presentation/contracts";
import type { FileLibrarySelectionSummary } from "../src/types/domain";

const t = makeTranslator("en");

const libraryTarget = {
  kind: "library" as const,
  source: "custom" as const,
  key: "w2-07"
};

function inspectorProps(overrides: Partial<FileLibraryInspectorProps> = {}): FileLibraryInspectorProps {
  return {
    selectedIds: new Set(["file-1"]),
    selectedFiles: [],
    selectionKind: "explicit",
    selectedCount: 1,
    detail: null,
    selectionSummary: null,
    isLoading: false,
    error: null,
    language: "en",
    t,
    onPreview: vi.fn(),
    onReveal: vi.fn(),
    onViewSuggestions: vi.fn(),
    onViewOperations: vi.fn(),
    onOpenContentUnderstanding: vi.fn(),
    onClearSelection: vi.fn(),
    onRetryDetail: vi.fn(),
    ...overrides
  };
}

function browseEntry(overrides: Partial<BrowsePresentationEntry> = {}): BrowsePresentationEntry {
  return {
    source: "browse",
    renderKey: "browse-entry" as BrowsePresentationEntry["renderKey"],
    entryRef: { kind: "ephemeral", browseSessionId: "session", entryId: "entry-1" },
    displayName: "report.txt",
    entryKind: "file",
    extension: ".txt",
    size: 42,
    materialization: "local",
    ...overrides
  };
}

describe("W2-07 Context Panel projection and visibility contracts", () => {
  it("keeps contextOpen bounded to presentation and out of navigation history", () => {
    const session = new WorkspaceSession({ initialTarget: libraryTarget });
    const before = session.getState();

    expect(session.setPresentation({ contextOpen: true })).toBe(true);
    const after = session.getState();
    expect(after.history).toEqual(before.history);
    expect(after.historyIndex).toBe(before.historyIndex);
    expect(after.presentation.contextOpen).toBe(true);

    const restore = session.serializeRestoreMetadata();
    expect(restore?.presentation?.contextOpen).toBe(true);
    expect(parseWorkspaceRestoreMetadata(restore)).toMatchObject({
      ok: true,
      metadata: { presentation: { contextOpen: true } }
    });
    expect(parseWorkspaceRestoreMetadata({
      version: 1,
      locator: { kind: "library", source: "custom", key: "w2-07" },
      presentation: { contextOpen: "yes" }
    })).toEqual({ ok: false, reason: "invalid_presentation" });
  });

  it("updates the controller preference without adding a navigation step", () => {
    const session = new WorkspaceSession({ initialTarget: libraryTarget });
    const controller = new FileWorkspaceController(undefined, session);
    const experience = new FileLibraryExperienceController(controller);
    const before = experience.getState().workspace.session;

    expect(experience.setContextOpen(true)).toBe(true);
    const after = experience.getState().workspace.session;
    expect(after.history).toEqual(before.history);
    expect(after.historyIndex).toBe(before.historyIndex);
    expect(after.presentation.contextOpen).toBe(true);
  });

  it("preserves viewMode and contextOpen independently through the experience controller", () => {
    const session = new WorkspaceSession({ initialTarget: libraryTarget });
    const controller = new FileWorkspaceController(undefined, session);
    const experience = new FileLibraryExperienceController(controller);

    expect(experience.setContextOpen(true)).toBe(true);
    expect(experience.setViewMode("grid")).toBe(true);
    expect(experience.getState().workspace.session.history).toHaveLength(1);
    expect(experience.getState().workspace.session.presentation).toEqual({ viewMode: "grid", contextOpen: true });

    expect(experience.setContextOpen(false)).toBe(true);
    expect(experience.getState().workspace.session.presentation).toEqual({ viewMode: "grid", contextOpen: false });

    expect(experience.setViewMode("list")).toBe(true);
    expect(experience.getState().workspace.session.presentation).toEqual({ viewMode: "list", contextOpen: false });
  });

  it("keeps Library single, explicit multi, and all_matching content states distinct", () => {
    const single = { kind: "explicit" as const, fileIds: ["file-1"] };
    const multi = { kind: "explicit" as const, fileIds: ["file-1", "file-2"] };
    const allMatching = {
      kind: "all_matching" as const,
      query: { scope: { kind: "all_enabled_roots" as const }, text: null, filters: { fileTypes: [], purposes: [], lifecycles: [], risks: [], tagsAllOf: [], tagsAnyOf: [], tagsNoneOf: [], sizeMin: null, sizeMax: null, modifiedFrom: null, modifiedTo: null, createdFrom: null, createdTo: null, duplicate: "any" as const, review: "any" as const }, sort: { kind: "name" as const, direction: "asc" as const } },
      queryFingerprint: "query",
      snapshotRevision: 1,
      excludedFileIds: []
    };
    const summary: FileLibrarySelectionSummary = {
      count: 100_000,
      totalSize: 123,
      typeCounts: [],
      missingCount: 0,
      staleCount: 0,
      excludedCount: 0,
      commonDirectory: null,
      commonTags: [],
      commonTagIds: [],
      partialTagCommonalityCount: 0,
      snapshotRevision: 1,
      queryFingerprint: "query"
    };

    expect(libraryContextContentKind(null)).toBe("none");
    expect(libraryContextContentKind(single)).toBe("inspector");
    expect(libraryContextContentKind(multi)).toBe("selection-summary");
    expect(libraryContextContentKind(allMatching)).toBe("selection-summary");
    expect(libraryContextSelectionCount(allMatching, summary)).toBe(100_000);
    expect(libraryContextSelectionCount(allMatching, null)).toBeNull();
    expect(createLibraryContextProjection(allMatching, inspectorProps({ selectionSummary: summary, selectedIds: new Set(["loaded-only"]) })).inspector.selectedCount).toBe(100_000);
  });

  it("uses LibrarySelectionV1 for explicit multi-selection even when only one row is loaded", () => {
    const selection = { kind: "explicit" as const, fileIds: ["A", "B"] };
    const projection = createLibraryContextProjection(selection, inspectorProps({ selectedIds: new Set(["A"]) }));

    expect(projection.inspector.selectedCount).toBe(2);
    expect(libraryInspectorContentKind(projection.inspector.selectionKind, projection.inspector.selectedCount)).toBe("selection-summary");
  });

  it("keeps an unloaded explicit single identity canonical for every detail load path", () => {
    const selection = { kind: "explicit" as const, fileIds: ["X"] };
    const loadDetail = vi.fn();
    const canonicalId = explicitSingleSelectionId(selection);

    if (canonicalId !== null) loadDetail(canonicalId);

    expect(canonicalId).toBe("X");
    expect(loadDetail).toHaveBeenCalledWith("X");
    expect(loadDetail).not.toHaveBeenCalledWith(undefined);
  });

  it("keeps all_matching compact for both zero and one loaded row", () => {
    const selection = {
      kind: "all_matching" as const,
      query: { scope: { kind: "all_enabled_roots" as const }, text: null, filters: { fileTypes: [], purposes: [], lifecycles: [], risks: [], tagsAllOf: [], tagsAnyOf: [], tagsNoneOf: [], sizeMin: null, sizeMax: null, modifiedFrom: null, modifiedTo: null, createdFrom: null, createdTo: null, duplicate: "any" as const, review: "any" as const }, sort: { kind: "name" as const, direction: "asc" as const } },
      queryFingerprint: "query",
      snapshotRevision: 1,
      excludedFileIds: []
    };

    for (const selectedIds of [new Set<string>(), new Set(["loaded-only"])]) {
      const projection = createLibraryContextProjection(selection, inspectorProps({ selectedIds, selectionSummary: null }));
      expect(projection.kind).toBe("selection-summary");
      expect(libraryInspectorContentKind(projection.inspector.selectionKind, projection.inspector.selectedCount)).toBe("selection-summary");
    }
  });

  it("does not infer whole-selection count from the loaded selected projection", () => {
    const selection = { kind: "explicit" as const, fileIds: ["A", "B"] };
    const projection = createLibraryContextProjection(selection, inspectorProps({ selectedIds: new Set(["A"]) }));

    expect(projection.inspector.selectedIds.size).toBe(1);
    expect(projection.inspector.selectedCount).toBe(2);
    expect(projection.inspector.selectedCount).not.toBe(projection.inspector.selectedIds.size);
  });

  it("keeps Browse summaries loaded-only and truthful when sizes are unknown", () => {
    const entries = [browseEntry(), browseEntry({ renderKey: "browse-entry-2" as BrowsePresentationEntry["renderKey"], entryRef: { kind: "ephemeral", browseSessionId: "session", entryId: "entry-2" }, displayName: "cloud.bin", size: undefined, materialization: "metadata_only" })];
    const projection = createBrowseContextProjection({
      entries,
      selectedIds: new Set(["entry-1", "entry-2"]),
      locationLabel: "Documents",
      language: "en",
      t
    });

    expect(projection.kind).toBe("selection-summary");
    expect(projection.selectedCount).toBe(2);
    expect(projection.size).toEqual({ state: "partial", total: 42 });
    expect(browseSizeProjection([browseEntry({ size: undefined })])).toEqual({ state: "unknown", total: null });
    expect(JSON.stringify(projection)).not.toContain("displayPath");
  });

  it("exposes one explicit Context command and removes the permanent Inspector layout", () => {
    const html = renderToStaticMarkup(createElement(WorkspaceCommandBar, {
      mode: "library",
      targetLabel: "File Library",
      canGoBack: false,
      canGoForward: false,
      onBack: vi.fn(),
      onForward: vi.fn(),
      viewMode: "grid",
      onViewModeChange: vi.fn(),
      contextOpen: true,
      onContextToggle: vi.fn(),
      t
    }));
    expect(html).toContain("data-file-library-view-mode=\"grid\"");
    expect(html).toContain("data-file-library-context-toggle=\"true\"");
    expect(html).toContain("aria-pressed=\"true\"");

    const workspace = read("src/views/fileLibrary/FileLibraryWorkspace.tsx");
    const libraryMode = read("src/views/fileLibrary/library/LibraryMode.tsx");
    const browseMode = read("src/views/fileLibrary/browse/BrowseMode.tsx");
    const grid = read("src/views/fileLibrary/list/SharedFileGrid.tsx");
    const panel = read("src/views/fileLibrary/context/ContextPanel.tsx");
    expect(workspace.match(/data-file-library-view-mode=/g)).toHaveLength(1);
    expect(libraryMode).toContain('viewMode === "grid" ? <SharedFileGrid');
    expect(browseMode).toContain('viewMode === "grid" ? <SharedFileGrid');
    expect(libraryMode.match(/onEscape={handleListEscape}/g)?.length).toBeGreaterThanOrEqual(2);
    expect(browseMode.match(/onEscape={handleListEscape}/g)?.length).toBeGreaterThanOrEqual(2);
    expect(grid).toContain("if (!onEscape?.()) interaction.actions.clearSelection();");
    expect(libraryMode).not.toContain("InspectorLayout");
    expect(libraryMode).toContain("ContextPanel");
    expect(browseMode).toContain("ContextPanel");
    expect(panel).not.toContain("displayPath");
    expect(panel).not.toContain("Preview Core");
  });
});

function read(path: string) {
  return readFileSync(resolve(path), "utf8");
}
