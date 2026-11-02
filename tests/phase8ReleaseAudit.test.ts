import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return /\.(ts|tsx|css)$/.test(entry.name) ? [path] : [];
  });
}

function read(path: string) {
  return readFileSync(resolve(path), "utf8");
}

describe("phase 8 release safety and polish contracts", () => {
  it("keeps native confirmation and fixed utility palette out of production source", () => {
    const source = sourceFiles(resolve("src")).map((path) => readFileSync(path, "utf8")).join("\n");

    expect(source).not.toMatch(/(?:globalThis|window)\.confirm\s*\(/);
    expect(source).not.toMatch(/\b(?:slate|blue|red|amber|emerald|violet|purple|green)-\d{2,3}\b/);
    expect(source).not.toMatch(/\.(?:only|skip)\s*\(/);
  });

  it("gates engineering diagnostics and keeps AI onboarding on readiness and consent authorities", () => {
    const settings = read("src/views/settings/SettingsView.tsx");
    const onboarding = read("src/components/OnboardingDialog.tsx");
    const cleanup = read("src/views/cleanup/StorageCleanupView.tsx");
    const cleanupExecution = read("src/views/cleanup/useCleanupExecutionController.ts");

    expect(settings).toContain('developerMode ? (');
    expect(settings).toContain('t("developerModeDesc")');
    expect(settings).toContain("tauriApi.saveAISettings");
    expect(onboarding).toContain("ModalPortal");
    expect(onboarding).toContain("getAIFeatureReadiness");
    // ADR-0016: the intro never configures a scan root, managed scope, saved
    // settings or a scan; Global Search consent goes through the dedicated
    // explicit-consent module instead.
    expect(onboarding).not.toContain("setDefaultScanFolders");
    expect(onboarding).not.toContain("addManagedScope");
    expect(onboarding).not.toContain("saveSettings");
    expect(onboarding).not.toContain("startManagedScan");
    expect(read("src/components/onboardingGlobalSearch.ts")).toContain("authorizeGlobalIndexSource");
    expect(onboarding).not.toContain("saveAISettings");
    expect(onboarding).not.toContain("getAISettings");
    expect(onboarding).not.toContain("listAIProviderPresets");
    expect(onboarding).not.toContain("moveFile");
    expect(onboarding).not.toContain("renameFile");
    expect(onboarding).not.toContain("delete");
    expect(cleanup).toContain("ConfirmDialog");
    expect(cleanupExecution).toContain("moveSelectedToSafeTrash");
    expect(cleanupExecution).toContain("previewCleanupOperations");
  });
});
