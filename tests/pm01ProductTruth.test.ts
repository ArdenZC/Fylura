import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";

const zh = makeTranslator("zh");
const en = makeTranslator("en");

describe("PM-01 product truth", () => {
  it("positions Rules and Automation as compatibility policies, not Organize semantic authority", () => {
    for (const [t, compatibilityPattern, legacyPattern, doesNotCreatePattern, noReplacePattern] of [
      [zh, /兼容性/, /旧分类/, /不会生成/, /不会替换/],
      [en, /legacy classification policies/, /legacy classification/, /does not create/, /do not replace/]
    ] as const) {
      expect(t("viewDescRules")).toMatch(compatibilityPattern);
      expect(t("automationWorkspaceDesc")).toMatch(/Managed AI/);
      expect(t("settingsAutomationDesc")).toMatch(/Managed AI/);
      expect(t("automationRunConfirmDesc")).toMatch(legacyPattern);
      expect(t("automationRunConfirmDesc")).toMatch(doesNotCreatePattern);
      expect(t("automationPreviewRequiredHint")).toMatch(/Managed AI/);
      expect(t("reapplyRulesSafetyDesc")).toMatch(noReplacePattern);
    }
  });

  it("keeps learning history out of current Managed AI semantics and makes no active Preference Memory claim", () => {
    const dictionary = readFileSync(resolve("src/i18n/dictionary.ts"), "utf8");
    expect(zh("aiLearningHint")).toContain("当前 Managed AI 语义评估不会读取它");
    expect(en("aiLearningHint")).toContain("Current Managed AI semantic assessments do not read it");
    expect(zh("aiLearnedRulesDesc")).toContain("不是当前 Managed AI 语义输入");
    expect(en("aiLearnedRulesDesc")).toContain("not an input to current Managed AI semantics");
    expect(dictionary).not.toMatch(/Preference Memory|偏好记忆/i);
  });

  it("keeps Files, Search, Preview, and History/Restore available without AI or a Managed Library scope", () => {
    expect(zh("onboardingFilesNoConfig")).toContain("不需要任何前置配置");
    expect(en("onboardingFilesNoConfig")).toContain("No setup required");
    expect(zh("onboardingSearchSkipHint")).toContain("暂不启用");
    expect(en("onboardingSearchSkipHint")).toContain("Skipping");
    for (const key of [
      "onboardingFilesTitle",
      "onboardingOrganizeTitle",
      "onboardingCleanupTitle",
      "onboardingSearchTitle"
    ] as const) {
      expect(zh(key).length).toBeGreaterThan(0);
      expect(en(key).length).toBeGreaterThan(0);
    }
  });

  it("no longer gates first launch on a folder, AI provider or index authorization", () => {
    const dictionary = readFileSync(resolve("src/i18n/dictionary.ts"), "utf8");
    // ADR-0016 supersedes the old required-useful-folder onboarding contract.
    expect(dictionary).not.toContain("onboardingUsefulFolderRequired");
    expect(dictionary).not.toContain("onboardingSkipAI");
    expect(dictionary).not.toContain("onboardingManagedConsentTitle");
    // AI-dependent scenes stay opt-in and point at the existing AI Settings.
    expect(zh("onboardingOrganizeNeedsAI")).toContain("单独授权");
    expect(en("onboardingOrganizeNeedsAI")).toContain("separate AI provider authorization");
    expect(zh("onboardingStartUsing").length).toBeGreaterThan(0);
    expect(en("onboardingStartUsing").length).toBeGreaterThan(0);
  });
});
