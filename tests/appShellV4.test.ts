import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

describe("App Shell v4", () => {
  const appShell = read("src/components/AppShell.tsx");
  const commandModal = read("src/components/CommandModal.tsx");
  const shellChrome = read("src/components/ShellChrome.tsx");
  const tokens = read("src/styles/tokens.css");

  it("uses the final primary and advanced navigation without removing routed views", () => {
    const nav = appShell.slice(appShell.indexOf("function navGroups"));

    for (const id of ["library", "organize", "cleanup", "restore", "scanner", "settings"]) {
      expect(nav).toContain(`id: "${id}"`);
    }
    expect(nav).not.toContain('id: "global-search"');
    expect(nav).not.toContain('id: "rules"');
    expect(nav).not.toContain('id: "preview"');
    expect(appShell).toContain('view === "cleanup"');
    expect(appShell).toContain('view === "preview"');
    expect(appShell).toContain('const AutomationWorkspace = lazy(() => import("../views/automation/AutomationWorkspace")');
    expect(appShell).toContain('else if (view === "automation") content = <AutomationWorkspace />');
    expect(appShell).not.toContain('view === "rules"');
    expect(read("src/views/automation/AutomationWorkspace.tsx")).toContain('import("../rules/RulesView")');
    expect(appShell).toContain('id: "primary"');
    expect(appShell).toContain('id: "advanced"');
  });

  it("uses the approved v3.40 shell geometry and semantic selected navigation", () => {
    const shell = read("src/styles/v340-shell.css");

    // One 44px glass titlebar, then two panels separated by the shell gutter.
    expect(appShell).toContain("zc-titlebar");
    expect(appShell).toContain("zc-shell-body");
    expect(appShell).toContain("data-sidebar-mode={sidebarMode}");
    expect(shell).toContain("--zc-titlebar-height");
    expect(shell).toContain("grid-template-rows: var(--zc-titlebar-height) minmax(0, 1fr)");
    expect(shell).toContain("grid-template-columns: var(--zc-shell-sidebar-width) minmax(0, 1fr)");
    expect(shell).toContain("gap: var(--zc-shell-gap)");
    expect(shell).toContain("--zc-glass-sidebar");
    expect(shell).toContain("--zc-glass-main");

    // Three reachable sidebar states, each with a stated next action.
    expect(appShell).toContain("resolveSidebarMode");
    expect(appShell).toContain("nextSidebarMode");
    expect(appShell).toContain('export type SidebarMode = "expanded" | "rail" | "hidden"');
    expect(shell).toContain('.zc-shell-body[data-sidebar-mode="rail"]');
    expect(shell).toContain('.zc-shell-body[data-sidebar-mode="hidden"]');

    expect(appShell).toContain("focusVisibleState");
    expect(appShell).not.toContain("before:w-0.5");
    expect(appShell).not.toMatch(/(?:hover|active):scale-/);
  });

  it("reports the active local or AI mode instead of making an absolute local-only claim", () => {
    expect(appShell).toContain("useAIProcessingModeStore");
    expect(appShell).toContain("resolveAIProcessingMode");
    expect(appShell).toContain('t("modeLoading")');
    expect(appShell).toContain('t("modeFailed")');
    expect(appShell).toContain('t("modeAICloud")');
    expect(appShell).toContain('t("modeAILocal")');
    expect(appShell).toContain('t("modeAIDisabled")');
    for (const icon of ["LoaderCircle", "TriangleAlert", "LockKeyhole", "Cpu", "Cloud"]) {
      expect(appShell).toContain(icon);
    }
    expect(appShell).toContain("openAIProcessingModeSettings");
    expect(appShell).toContain('openSection("settings-ai")');
  });

  it("does not render permanent theme or language controls in the titlebar", () => {
    expect(appShell).not.toContain("<TitlebarTools");
    expect(appShell).not.toContain("function ChromeTools");
    expect(appShell).not.toContain("setLanguage");
    expect(appShell).not.toContain("setTheme");
    expect(appShell).toContain("MacWindowControls");
    expect(appShell).toContain("WindowsControls");
  });

  it("keeps background content inert, restores Spotlight focus, and does not inject scan actions", () => {
    expect(appShell).toContain("APP_SHELL_CONTENT_ID");
    expect(appShell).toContain("<ModalHost />");
    expect(appShell).not.toContain("inert={isCommandOpen ? true : undefined}");
    expect(appShell).toContain("restoreFocusRef={spotlightTriggerRef}");
    expect(appShell).not.toContain("handleChooseFolders");
    expect(appShell).not.toContain("handleScan");
    expect(appShell).not.toContain("FolderSearch");
    expect(appShell).not.toContain("RefreshCw");
  });

  it("uses semantic success colors without fixed AppShell status palettes", () => {
    expect(appShell).toContain("var(--zc-success-border)");
    expect(appShell).toContain("var(--zc-success-soft)");
    expect(appShell).toContain("var(--zc-success-text)");
    expect(appShell).not.toMatch(/(?:emerald|red|blue)-\d/);
  });

  it("keeps platform controls native to each platform and drag-safe", () => {
    const shell = read("src/styles/v340-shell.css");
    expect(appShell).toContain("MacWindowControls");
    expect(appShell).toContain("WindowsControls");
    // macOS traffic lights lead; the Windows cluster trails (DESIGN-SPEC §4.2).
    const titlebarLead = appShell.slice(appShell.indexOf("zc-titlebar-lead"), appShell.indexOf("zc-titlebar-trail"));
    expect(titlebarLead).toContain("isWindows && <MacWindowControls");
    expect(appShell).toContain("{isWindows && <WindowsControls");
    expect(shell).toContain(".zc-mac-controls");
    expect(shell).toContain(".zc-window-controls");
    expect(shell).toContain("var(--zc-window-close-hover)");
    expect(shell).toContain("var(--zc-window-mac-close)");
    expect(appShell).not.toContain("overflow-hidden rounded-lg border");
    expect(appShell).toContain("[-webkit-app-region:no-drag]");
    expect(shellChrome).toContain("[-webkit-app-region:no-drag]");
    expect(tokens).toContain("--zc-window-close-hover");
  });

  it("uses the floating material and semantic colors for Spotlight", () => {
    expect(commandModal).toContain("var(--zc-surface-overlay)");
    expect(commandModal).toContain("var(--zc-shadow-spotlight)");
    expect(commandModal).toContain("focusVisibleState");
    expect(commandModal).toContain("selectedFocusSurface");
    expect(commandModal).toContain("commandIdleGroups");
    expect(commandModal).toContain("isBackgroundIndexing");
    expect(commandModal).not.toMatch(/(?:neutral|blue|slate|red|amber|emerald|green|purple)-\d/);
    expect(commandModal).not.toMatch(/(?:hover|active):scale-/);
  });

  it("preserves Spotlight accessibility and keyboard behavior", () => {
    expect(commandModal).toContain('role={standalone ? "search" : "dialog"}');
    expect(commandModal).toContain("aria-modal={standalone ? undefined : true}");
    expect(commandModal).toContain('if (event.key === "ArrowDown")');
    expect(commandModal).toContain('if (event.key === "ArrowUp")');
    expect(commandModal).toContain('if (event.key === "Enter"');
    expect(commandModal).toContain("onEscape={closeSpotlight}");
    expect(commandModal).toContain("initialFocusRef={inputRef}");
    expect(commandModal).toContain("isSortingPreviewShortcut(event)");
  });
});
