// @vitest-environment happy-dom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { WorkspaceSession } from "../src/fileWorkspace";
import { makeTranslator } from "../src/i18n";
import { WorkspaceCommandBar } from "../src/views/fileLibrary/FileLibraryWorkspace";

const t = makeTranslator("en");

describe("W2-08 search, sort and presentation contracts", () => {
  it("mounts one caller-provided local search surface in the canonical command bar", () => {
    const html = renderToStaticMarkup(createElement(WorkspaceCommandBar, {
      mode: "library",
      targetLabel: t("fileLibrary"),
      canGoBack: false,
      canGoForward: false,
      onBack: vi.fn(),
      onForward: vi.fn(),
      localSearch: createElement("input", {
        "data-file-library-local-search": "true",
        "aria-label": "Search the File Library"
      }),
      t
    }));

    expect(html).toContain('data-file-library-command-search="true"');
    expect(html.match(/data-file-library-local-search="true"/gu)).toHaveLength(1);
  });

  it("keeps presentation changes out of workspace navigation history", () => {
    const session = new WorkspaceSession({
      initialTarget: { kind: "library", source: "custom", key: "w2-08" }
    });
    const before = session.getState();

    expect(session.setPresentation({ viewMode: "grid" })).toBe(true);
    const after = session.getState();
    expect(after.history).toEqual(before.history);
    expect(after.historyIndex).toBe(before.historyIndex);
    expect(after.presentation.viewMode).toBe("grid");
  });

  it("exposes Browse current-folder search and an honest whole-folder sort boundary", () => {
    const browseMode = readFileSync(resolve("src/views/fileLibrary/browse/BrowseMode.tsx"), "utf8");
    const browseSourceOwner = readFileSync(resolve("src/views/fileLibrary/browse/browseSourceOwner.ts"), "utf8");
    const workspace = readFileSync(resolve("src/views/fileLibrary/FileLibraryWorkspace.tsx"), "utf8");

    expect(browseMode).toContain('t("browseSearchPlaceholder")');
    expect(browseMode).toContain('source.setQueryText');
    expect(browseMode).toContain('data-browse-sort-capability="backend-snapshot"');
    expect(browseMode).toContain('data-browse-query-empty-partial={emptyPartialQuery ? "true" : "false"}');
    expect(browseMode).toContain('t("browseEnumerationSearching")');
    expect(browseSourceOwner).toContain("window.setTimeout");
    expect(browseSourceOwner).toContain("window.clearTimeout");
    expect(browseSourceOwner).toContain('entries.length === 0');
    expect(browseMode).not.toContain('data-file-library-local-search-state="unavailable"');
    expect(workspace).toContain('event.key.toLowerCase() !== "f"');
    expect(workspace).toContain("event.isComposing");
  });
});
