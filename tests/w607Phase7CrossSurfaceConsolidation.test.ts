import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

describe("W6-07 Phase 7 cross-surface consolidation", () => {
  it("exposes one shared focus and selection state grammar for controls and rows", () => {
    const tw = read("src/utils/tw.ts");
    const shared = read("src/views/shared/ui.ts");
    const switches = read("src/components/ui/Switch.tsx");

    for (const helper of ["focusVisibleState", "focusWithinSurface", "selectedSurface", "selectedFocusSurface"]) {
      expect(tw).toContain(`export const ${helper}`);
    }
    expect(tw).toContain("focus-within:shadow-none");
    expect(tw).not.toContain("focus:shadow-[0_0_0_3px_var(--zc-focus-ring-soft)]");
    expect(tw).not.toContain("ring-1 ring-[var(--zc-");
    expect(shared).toContain("active && cn(selectedSurface, \"font-semibold\")");
    expect(shared).not.toContain("shadow-[inset_3px_0_0_var(--zc-primary)]");
    expect(shared).not.toContain("shadow-[inset_0_1px_0_var(--zc-brand-canvas-highlight)]");
    expect(switches).toContain("h-5 w-9");
    expect(switches).toContain("left-[2px]");
    expect(switches).toContain("h-3.5 w-3.5");
    expect(switches).toContain("bg-[var(--zc-primary-soft)]");
    expect(switches).not.toContain("shadow-inner");
    expect(switches).not.toContain("shadow-[0_2px_8px_var(--zc-primary-soft)]");
  });

  it("keeps migrated surfaces free of focus rails, selection glows and raw overlays", () => {
    const sources = [
      "src/components/AppShell.tsx",
      "src/components/CommandModal.tsx",
      "src/components/ShellChrome.tsx",
      "src/views/history/HistoryBatchList.tsx",
      "src/views/history/HistorySearchField.tsx",
      "src/views/organize/OrganizeSuggestionList.tsx",
      "src/views/organize/OrganizeSuggestionsView.tsx",
      "src/views/rules/AutomationRuleList.tsx",
      "src/views/restore/RestoreView.tsx",
      "src/views/settings/components/SettingsPrimitives.tsx",
      "src/views/settings/sections/GlobalSearchSettingsSection.tsx",
      "src/views/overview/OverviewSections.tsx",
      "src/views/vault/AssetCard.tsx",
      "src/views/vault/components/FileLibraryInspector.tsx",
      "src/views/vault/components/ContentUnderstandingSheet.tsx",
      "src/views/organize/OrganizeTargetDialog.tsx"
    ];

    for (const path of sources) {
      const source = read(path);
      expect(source, `${path} retains an inset selection rail`).not.toContain("shadow-[inset");
      expect(source, `${path} retains a focus glow`).not.toMatch(/focus(?:-within|-visible)?:shadow-\[/);
      expect(source, `${path} retains a raw black overlay`).not.toContain("bg-black/");
    }
  });

  it("preserves the single Files destination and explicit route ownership", () => {
    const shell = read("src/components/AppShell.tsx");
    expect(shell).toContain('id: "library"');
    expect(shell).toContain('<FileLibraryWorkspace active={view === "library"}/>');
    expect(shell).toContain('hidden={view !== "library"}');
    expect(shell).toContain('else if (view === "automation") content = <AutomationWorkspace />');
    expect(shell).toContain('else if (view === "restore") content = <RestoreView />');
    expect(shell).toContain("projectAcceptedFileLibraryActivation");
  });
});
