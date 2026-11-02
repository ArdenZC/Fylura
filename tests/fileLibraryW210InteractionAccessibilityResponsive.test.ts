// @vitest-environment happy-dom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { WorkspaceSession } from "../src/fileWorkspace/workspaceSession";
import { makeTranslator } from "../src/i18n";
import { WorkspaceCommandBar } from "../src/views/fileLibrary/FileLibraryWorkspace";
import { FileLibraryContextMenu } from "../src/views/fileLibrary/library/LibraryContextMenu";
import {
  isFileLibraryFocusTarget,
  isFileLibraryShortcutExcludedTarget
} from "../src/views/fileLibrary/fileLibraryInteraction";

const t = makeTranslator("en");
const read = (file: string) => readFileSync(resolve(process.cwd(), file), "utf8");

describe("W2-10 interaction/accessibility/responsive integration contracts", () => {
  it("lets editing and dialog-owned controls keep Cmd/Ctrl+F", () => {
    const input = document.createElement("input");
    const textarea = document.createElement("textarea");
    const select = document.createElement("select");
    const editable = document.createElement("div");
    editable.contentEditable = "true";
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    const dialogButton = document.createElement("button");
    dialog.append(dialogButton);
    const ordinaryButton = document.createElement("button");

    expect(isFileLibraryShortcutExcludedTarget(input)).toBe(true);
    expect(isFileLibraryShortcutExcludedTarget(textarea)).toBe(true);
    expect(isFileLibraryShortcutExcludedTarget(select)).toBe(true);
    expect(isFileLibraryShortcutExcludedTarget(editable)).toBe(true);
    expect(isFileLibraryShortcutExcludedTarget(dialogButton)).toBe(true);
    expect(isFileLibraryShortcutExcludedTarget(ordinaryButton)).toBe(false);
  });

  it("keeps command-bar target, navigation and context state exposed", () => {
    const html = renderToStaticMarkup(createElement(WorkspaceCommandBar, {
      mode: "browse",
      targetLabel: "Work folder",
      canGoBack: true,
      canGoForward: false,
      onBack: vi.fn(),
      onForward: vi.fn(),
      navigationOpen: false,
      onNavigationToggle: vi.fn(),
      contextOpen: true,
      onContextToggle: vi.fn(),
      t
    }));

    expect(html).toContain('role="toolbar"');
    expect(html).toContain('aria-controls="file-library-navigation-slot"');
    expect(html).toContain('aria-expanded="false"');
    expect(html).toContain('aria-expanded="true"');
    expect(html).toContain('aria-label="Work folder"');
    // ADR-0016 removed the Library/Browse tablist; the command bar keeps one
    // presented mode plus pressed-state view and context toggles.
    expect(html).toContain('data-file-library-presented-mode="browse"');
    expect(html).not.toContain('role="tab"');
    expect(html).toContain('aria-pressed="true"');
  });

  it("uses one bounded menu keyboard owner for source context menus", () => {
    const html = renderToStaticMarkup(createElement(FileLibraryContextMenu, {
      x: 12,
      y: 18,
      title: "notes.txt",
      ariaLabel: "Browse item menu",
      items: [
        { label: "Open context", action: vi.fn() },
        { label: "Clear selection", action: vi.fn() }
      ],
      onClose: vi.fn()
    }));

    expect(html).toContain('role="menu"');
    expect(html).toContain('aria-label="Browse item menu"');
    expect(html.match(/role="menuitem"/g)?.length).toBe(2);
    expect(html).toContain("Open context");
    expect(html).toContain("Clear selection");
  });

  it("keeps focus fallback source-owned and removes delayed local focus chains", () => {
    const list = document.createElement("div");
    list.tabIndex = 0;
    document.body.append(list);
    expect(isFileLibraryFocusTarget(list)).toBe(true);

    const libraryMode = read("src/views/fileLibrary/library/LibraryMode.tsx");
    const contentCompatibility = read("src/views/fileLibrary/library/useLibraryContentCompatibility.ts");
    expect(libraryMode).not.toContain("previewOpenEpoch.current !== closeEpoch");
    expect(libraryMode).not.toContain("previewOpenEpoch.current === closeEpoch");
    expect(contentCompatibility).not.toContain("requestAnimationFrame");
  });

  it("keeps Browse partial ARIA truthful and wires both item context-menu paths", () => {
    const browseMode = read("src/views/fileLibrary/browse/BrowseMode.tsx");
    const grid = read("src/views/fileLibrary/list/SharedFileGrid.tsx");
    expect(browseMode).toContain("onOpenContextMenu={openFocusedContextMenu}");
    expect(browseMode).toContain("handleRowContextMenu(event, entry)");
    expect(browseMode).toContain('ariaLabel={t("browseContextMenu")}');
    expect(grid).toContain('interaction.source === "browse" && interaction.hasMore ? undefined : rowCount');
    expect(grid).toContain("aria-colindex={columnIndex + 1}");
  });

  it("keeps compact overlay policy and reduced-motion behavior explicit", () => {
    const workspace = read("src/views/fileLibrary/FileLibraryWorkspace.tsx");
    const workspaceCss = read("src/views/fileLibrary/fileLibraryWorkspace.css");
    const browseCss = read("src/views/fileLibrary/browse/browseMode.css");
    expect(workspace).toContain("if (nextOpen && layout !== \"large\" && contextOpen) controller.setContextOpen(false)");
    expect(workspace).toContain("if (nextOpen && layout !== \"large\") setNavigationOpen(false)");
    expect(workspaceCss).toContain("@media (prefers-reduced-motion: reduce)");
    expect(browseCss).toContain("@media (prefers-reduced-motion: reduce)");
  });

  it("restores Browse query presentation through WorkspaceSession Back/Forward", () => {
    const browseTarget = {
      kind: "browse" as const,
      location: { kind: "ephemeral" as const, browseSessionId: "session-w210", locationId: "location-w210" },
      pathRef: { id: "root-w210" }
    };
    const libraryTarget = { kind: "library" as const, source: "custom" as const, key: "library-w210" };
    const query = { text: "notes", entryKind: "file" as const };
    const session = new WorkspaceSession({ initialTarget: browseTarget });

    expect(session.setPresentation({ browseQuery: query })).toBe(true);
    expect(session.navigate(libraryTarget)).toBe(true);
    expect(session.getState().presentation.browseQuery).toBeUndefined();
    expect(session.back()).toBe(true);
    expect(session.getState().presentation.browseQuery).toEqual(query);
    expect(session.forward()).toBe(true);
    expect(session.getState().presentation.browseQuery).toBeUndefined();
  });
});
