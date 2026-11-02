// @vitest-environment happy-dom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { navGroups } from "../src/components/AppShell";
import {
  createCommandRegistry,
  executeSpotlightCommand,
  queryCommandRegistry
} from "../src/components/spotlight/commandRegistry";
import { makeTranslator } from "../src/i18n";
import {
  focusLocalFileLibrarySearch,
  layoutForWidth,
  WorkspaceCommandBar
} from "../src/views/fileLibrary/FileLibraryWorkspace";
import { restoreFileLibraryFocus } from "../src/views/fileLibrary/fileLibraryInteraction";

const read = (file: string) => readFileSync(resolve(file), "utf8");

describe("W6-07 Phase 1B Files identity and continuity", () => {
  it("keeps exactly one global Files destination and routes its command to the existing library view", () => {
    for (const [language, label] of [["en", "Files"], ["zh", "文件"]] as const) {
      const t = makeTranslator(language);
      const items = navGroups(t).flatMap((group) => group.items);
      const filesItems = items.filter((item) => item.id === "library");

      expect(filesItems).toHaveLength(1);
      expect(filesItems[0]?.label).toBe(label);
      expect(items.some((item) => String(item.id) === "browse")).toBe(false);

      const command = createCommandRegistry(t).find((item) => item.id === "library");
      expect(command?.label).toBe(label);
      expect(queryCommandRegistry(label, createCommandRegistry(t))).toContainEqual(command);

      const setView = vi.fn();
      const requestSettingsSection = vi.fn();
      const onClose = vi.fn();
      if (!command) throw new Error("Files command is missing");
      executeSpotlightCommand(command, { setView, requestSettingsSection, onClose });
      expect(setView).toHaveBeenCalledWith("library");
      expect(requestSettingsSection).not.toHaveBeenCalled();
      expect(onClose).toHaveBeenCalledOnce();
    }

    for (const script of ["scripts/runW2-01BrowserGate.mjs", "scripts/runW2-10BrowserGate.mjs", "scripts/runW2-11BrowserGate.mjs"]) {
      const source = read(script);
      expect(source).toContain('name: "Files"');
      expect(source).not.toContain('name: "File Library"');
      if (script !== "scripts/runW2-10BrowserGate.mjs") {
        expect(source).toContain('name: "Browse Folder"');
        expect(source).not.toContain('name: "Browse", exact: true');
      }
    }
  });

  it("presents a single user-visible Files mode without changing route identifiers", () => {
    const english = renderToStaticMarkup(createElement(WorkspaceCommandBar, {
      mode: "browse",
      targetLabel: makeTranslator("en")("fileLibraryNoFolderOpen"),
      canGoBack: false,
      canGoForward: false,
      onBack: vi.fn(),
      onForward: vi.fn(),
      t: makeTranslator("en")
    }));
    const chinese = renderToStaticMarkup(createElement(WorkspaceCommandBar, {
      mode: "browse",
      targetLabel: makeTranslator("zh")("fileLibraryNoFolderOpen"),
      canGoBack: false,
      canGoForward: false,
      onBack: vi.fn(),
      onForward: vi.fn(),
      t: makeTranslator("zh")
    }));

    expect(english).toContain("No folder open");
    expect(chinese).toContain("未打开文件夹");
    // ADR-0016: the ordinary file manager no longer offers a Library/Browse switch.
    expect(english).not.toContain(">Library<");
    expect(english).not.toContain(">Browse Folder<");
    expect(english).not.toContain('role="tablist"');
    expect(chinese).not.toContain(">资料库<");
    expect(chinese).not.toContain(">浏览文件夹<");
    expect(english).toContain('data-file-library-presented-mode="browse"');

    const workspace = read("src/views/fileLibrary/FileLibraryWorkspace.tsx");
    // Managed Library implementation is retained, just not a presented mode.
    expect(workspace).toContain('state.mode === "library"');
    expect(workspace).toContain('import("./library/LibraryMode")');
    expect(workspace).toContain('<BrowseMode />');
    expect(workspace).toContain("fixtureControls={isManagedLibraryBrowserFixture() &&");
    expect(english).not.toContain("file-library-mode-switch");
    expect(workspace).not.toContain('import("./browse/BrowseMode")\nconst LibraryMode');
  });

  it("focuses the active source-owned local search for Ctrl/Cmd+F while leaving global search independent", () => {
    const sourceSurface = document.createElement("div");
    sourceSurface.tabIndex = 0;
    const libraryInput = document.createElement("input");
    const browseInput = document.createElement("input");
    document.body.append(sourceSurface, libraryInput, browseInput);

    for (const [input, modifier] of [[libraryInput, "ctrlKey"], [browseInput, "metaKey"]] as const) {
      const select = vi.spyOn(input, "select");
      let handled = false;
      const listener = (event: KeyboardEvent) => {
        handled = focusLocalFileLibrarySearch(event, {
          enabled: true,
          searchInputRef: { current: input }
        });
      };
      sourceSurface.addEventListener("keydown", listener);
      const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "f", [modifier]: true });
      sourceSurface.dispatchEvent(event);
      sourceSurface.removeEventListener("keydown", listener);

      expect(handled).toBe(true);
      expect(document.activeElement).toBe(input);
      expect(select).toHaveBeenCalledOnce();
      expect(event.defaultPrevented).toBe(true);
    }

    let excludedHandled = false;
    const excludedListener = (event: KeyboardEvent) => {
      excludedHandled = focusLocalFileLibrarySearch(event, {
        enabled: true,
        searchInputRef: { current: browseInput }
      });
    };
    browseInput.addEventListener("keydown", excludedListener);
    browseInput.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "f", ctrlKey: true }));
    browseInput.removeEventListener("keydown", excludedListener);
    expect(excludedHandled).toBe(false);

    sourceSurface.remove();
    libraryInput.remove();
    browseInput.remove();
  });

  it("uses the component-owned width state for large, medium, and compact Files layouts", () => {
    expect(layoutForWidth(1120)).toBe("large");
    expect(layoutForWidth(1119)).toBe("medium");
    expect(layoutForWidth(820)).toBe("medium");
    expect(layoutForWidth(819)).toBe("compact");
    expect(read("src/views/fileLibrary/FileLibraryWorkspace.tsx")).toContain("element.clientWidth");
    expect(read("src/views/fileLibrary/FileLibraryWorkspace.tsx")).not.toContain("window.innerWidth");
  });

  it("restores focus after a large or pane-scoped Inspector close and keeps the Forced Colors cue on local search", () => {
    const toggle = document.createElement("button");
    document.body.append(toggle);
    expect(restoreFileLibraryFocus(() => toggle)).toBe(true);
    expect(document.activeElement).toBe(toggle);

    toggle.disabled = true;
    expect(restoreFileLibraryFocus(() => toggle)).toBe(false);

    const panel = read("src/views/fileLibrary/context/ContextPanel.tsx");
    const css = read("src/views/fileLibrary/fileLibraryV26.css");
    expect(panel).toContain("if (layout === \"large\" || usePaneOverlay) restoreFileLibraryFocus(restoreFocus)");
    expect(css).toContain(".file-library-command-search input:focus-visible");
    expect(css).toContain("outline: 2px solid CanvasText");
    expect(css).toContain(".file-library-workspace .file-library-command-search input:focus-visible,");
    toggle.remove();
  });

  it("keeps Inspector and selection presentation adapters on their existing source authorities", () => {
    const library = read("src/views/fileLibrary/library/LibraryMode.tsx");
    const browse = read("src/views/fileLibrary/browse/BrowseMode.tsx");
    const projection = read("src/views/fileLibrary/context/contextPanelProjection.ts");

    expect(library).toContain("createLibraryInteractionProjection(source)");
    expect(library).toContain("createLibraryContextProjection(source.selection");
    expect(library).toContain("<ContextPanel");
    expect(browse).toContain("createBrowseInteractionProjection(source)");
    expect(browse).toContain("createBrowseContextProjection");
    expect(browse).toContain("<ContextPanel");
    expect(projection).toContain('selection.kind === "all_matching"');
    expect(projection).toContain('selectedCount === 1 ? "inspector" : "selection-summary"');
    expect(library).toContain('data-library-selection-authority="library-selection-v1"');
    expect(browse).toContain('data-browse-selection-authority="browse-source-local"');
  });
});
