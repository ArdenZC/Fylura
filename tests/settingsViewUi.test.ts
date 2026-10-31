import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

const settingsSectionPaths = [
  "src/views/settings/sections/GeneralSettingsSection.tsx",
  "src/views/settings/sections/FileSourcesSettingsSection.tsx",
  "src/views/settings/sections/GlobalSearchSettingsSection.tsx",
  "src/views/settings/sections/GlobalIndexSettingsSection.tsx",
  "src/views/settings/sections/PlatformDiagnosticsSettingsSection.tsx",
  "src/views/settings/sections/ManagedLibrarySettingsSection.tsx",
  "src/views/settings/sections/AutomationSettingsSection.tsx",
  "src/views/settings/sections/AISettingsSection.tsx",
  "src/views/settings/sections/PrivacyContentSettingsSection.tsx",
  "src/views/settings/sections/AboutSettingsSection.tsx",
  "src/views/settings/sections/DeveloperDiagnosticsSection.tsx"
] as const;

describe("settings view UI", () => {
  it("uses system-preferences sections and shared settings primitives", () => {
    const settingsView = read("src/views/settings/SettingsView.tsx");
    const settingsNavigation = read("src/views/settings/controllers/useSettingsNavigationController.ts");
    const settingsModel = read("src/views/settings/settingsSectionModel.ts");
    const settingsSections = settingsSectionPaths.map(read).join("\n");
    const settingsSurface = `${settingsView}\n${settingsSections}`;
    const settingsPrimitives = read("src/views/settings/components/SettingsPrimitives.tsx");
    const sharedUi = read("src/views/shared/ui.ts");
    const appShell = read("src/components/AppShell.tsx");
    const t = makeTranslator("zh");

    expect(t("settingsAppearanceLanguage")).toBe("外观与语言");
    expect(t("settingsScanRoots")).toBe("扫描目录");
    expect(t("settingsSearch")).toBe("搜索");
    expect(t("settingsOrganizeRoot")).toBe("整理目标根目录");
    expect(t("organizeRootCurrentFolder")).toBe("当前文件夹下分类");
    expect(t("organizeRootZenCanvasFolder")).toBe("集中放入专用整理文件夹");
    expect(t("organizeRootCustomRoot")).toBe("自定义整理目录");
    expect(t("settingsSafetyRestore")).toBe("安全与恢复");
    expect(t("settingsWindowBehavior")).toBe("窗口行为");
    expect(t("settingsStartup")).toBe("启动项");
    expect(t("settingsDeveloperRelease")).toBe("开发检查");

    expect(sharedUi).toContain("ControlGroup");
    expect(sharedUi).toContain("SwitchButton");
    expect(sharedUi).toContain("SwitchField");
    expect(sharedUi).toContain("SegmentedControl");
    expect(settingsView).toContain("SettingsLayout");
    expect(settingsView).toContain("SettingsSection");
    expect(settingsView).toContain("SettingsControlGroup");
    expect(settingsView).toContain("SettingsSegmentedControl");
    expect(settingsView).toContain("SettingsSwitch");
    expect(settingsPrimitives).toContain('data-settings-scroll-container');
    expect(settingsPrimitives).toContain('data-settings-layout-grid');
    expect(settingsPrimitives).toContain('min-[841px]:grid-cols-[200px_minmax(0,1fr)]');
    expect(settingsPrimitives).toContain('max-w-[1240px]');
    expect(settingsPrimitives).toContain('role="radiogroup"');
    expect(settingsPrimitives).toContain('role="switch"');
    expect(settingsPrimitives).toContain("data-settings-switch-track");
    expect(settingsPrimitives).toContain("data-settings-switch-thumb");
    expect(settingsPrimitives).toContain('aria-current={active ? "location" : undefined}');
    expect(settingsPrimitives).toContain("centerSettingsNavItem");
    expect(settingsPrimitives).not.toContain('scrollIntoView({ block: "nearest", inline: "nearest" })');
    expect(settingsPrimitives).toContain("[scrollbar-width:none]");
    expect(settingsPrimitives).toContain('data-settings-nav-fade="end"');
    expect(settingsPrimitives).toContain("sticky top-0 z-20");
    expect(settingsPrimitives).toContain("min-[841px]:grid-cols-[minmax(0,1fr)_minmax(0,360px)]");
    expect(settingsPrimitives).toContain("min-[841px]:grid-cols-[minmax(220px,1fr)_minmax(0,480px)]");
    expect(settingsPrimitives).not.toContain("min-[720px]:grid-cols");
    expect(settingsPrimitives).toContain("data-settings-progressive-disclosure");
    expect(settingsPrimitives).toContain("options.revealContent");
    expect(settingsModel).toContain('"settings-global-index"');
    expect(settingsModel).toContain('"settings-platform-diagnostics"');
    expect(settingsModel).toContain('"settings-managed-scopes"');
    expect(settingsModel).toContain('return settingsSectionRequestTarget(sectionId);');
    expect(settingsModel).toContain('export const SETTINGS_NAV_SECTION_IDS = SETTINGS_SECTION_IDS;');
    expect(settingsSurface).toContain('progressiveDisclosure');
    expect(settingsSurface).toContain('t("settingsAppearanceLanguage")');
    expect(settingsSurface).toContain('t("settingsScanRoots")');
    expect(settingsSurface).toContain('t("settingsSearch")');
    expect(settingsSurface).toContain('t("settingsOrganizeRoot")');
    expect(settingsSurface).toContain("organizeRootMode");
    expect(settingsSurface).toContain("setOrganizeRootMode");
    expect(settingsSurface).toContain("setOrganizeRootPath");
    expect(settingsSurface).toContain('t("organizePreviewStillRequired")');
    expect(settingsSurface).toContain('t("settingsPrivacy")');
    expect(settingsSurface).toContain('t("settingsWindowBehavior")');
    expect(settingsSurface).toContain('t("settingsStartup")');
    expect(settingsSurface).toContain('t("developerMode")');
    expect(settingsView).toContain("settingsSectionsLabel");
    expect(settingsSurface).toContain("href={packageInfo.homepage}");
    expect(settingsSurface).toContain('t("aboutOpenProject")');
    expect(settingsView).not.toContain("<h1");
    expect(appShell).toContain("ShellViewHeading");
    const sectionIds = [
      "settings-general",
      "settings-files-scan",
      "settings-search",
      "settings-global-index",
      "settings-platform-diagnostics",
      "settings-managed-scopes",
      "settings-automation",
      "settings-ai",
      "settings-privacy",
      "settings-about"
    ];
    for (const id of sectionIds) expect(settingsSurface).toContain(`id=\"${id}\"`);
    const sectionImports = [
      "AboutSettingsSection",
      "AISettingsSection",
      "AutomationSettingsSection",
      "FileSourcesSettingsSection",
      "GeneralSettingsSection",
      "GlobalIndexSettingsSection",
      "GlobalSearchSettingsSection",
      "ManagedLibrarySettingsSection",
      "PrivacyContentSettingsSection",
      "PlatformDiagnosticsSettingsSection",
      "DeveloperDiagnosticsSection",
    ].map((name) => settingsView.indexOf(`import { ${name} }`));
    expect(sectionImports.every((index) => index >= 0)).toBe(true);
    expect(sectionImports).toEqual([...sectionImports].sort((left, right) => left - right));
    expect(settingsNavigation).toContain('useState("settings-general")');
    expect(settingsNavigation).toContain("settingsSectionRequestTarget");
    expect(settingsNavigation).toContain("settingsNavigationSectionId");
    expect(settingsNavigation).toContain("isProgressiveSettingsSectionId");
    expect(settingsSurface).toContain('id="settings-general"');
    expect(settingsView).not.toContain("AppearanceSettingsSection");
    expect(settingsModel).not.toContain('"settings-appearance"');
    expect(settingsSurface).toContain('id="settings-language"');
    expect(settingsSurface).toContain('id="settings-files-scan"');
    expect(settingsSurface).toContain('id="settings-automation"');
    expect(settingsSurface).toContain('id="settings-ai"');
    expect(settingsSurface).toContain('id="settings-privacy"');
    expect(settingsSurface).toContain('id="settings-about"');
    expect(settingsView).not.toContain("statusToast");
  });

  it("polishes hotkey capture, directory rows, and developer release affordance", () => {
    const settingsView = read("src/views/settings/SettingsView.tsx");
    const settingsNavigation = read("src/views/settings/controllers/useSettingsNavigationController.ts");
    const settingsSurface = `${settingsView}\n${settingsSectionPaths.map(read).join("\n")}`;
    const t = makeTranslator("zh");

    expect(t("hotkeyCaptureTitle")).toBe("正在录制快捷键");
    expect(t("hotkeyCaptureCurrent")).toBe("当前按键");
    expect(t("settingsSavedInline")).toBe("已保存");
    expect(t("developerReleaseDesc")).toContain("开发用途");
    expect(t("confirmDeleteScanFolderTitle")).toBe("删除这个扫描目录？");
    expect(t("confirmDeleteSearchFolderTitle")).toBe("删除这个搜索目录？");

    expect(settingsView).toContain("recordingHotkeyPreview");
    expect(settingsView).not.toContain("statusLabel={root.enabled ? t(\"enabled\") : t(\"disabled\")}");
    expect(settingsView).not.toContain("className={toggleSwitch(root.enabled)}");
    expect(settingsSurface).toContain("SettingsInlineMessage");
    expect(settingsSurface).toContain("SettingsEmptyState");
    expect(settingsSurface).toContain("compactPath(root.path");
    expect(settingsSurface).toContain('aria-label={t("deleteScanFolder")}');
    expect(settingsSurface).toContain('title={t("deleteScanFolder")}');
    expect(settingsSurface).toContain('aria-label={t("deleteSearchFolder")}');
    expect(settingsSurface).toContain('title={t("deleteSearchFolder")}');
    expect(settingsView).toContain("ConfirmDialog");
    expect(settingsView).toContain("folderDeleteConfirm");
    expect(settingsView).toContain("if (saved) setFolderDeleteConfirm(null)");
    expect(settingsView).toContain("async function pickFolder(title: string)");
    expect(settingsView).toContain('t("folderPickerFailed")');
    expect(settingsSurface).toContain("aria-pressed={searchHotkey === accelerator}");
    expect(settingsView).toContain('t("confirmDeleteScanFolderDesc")');
    expect(settingsView).toContain('t("confirmDeleteSearchFolderDesc")');
    expect(settingsSurface).toContain("SettingsDisclosure");
    expect(settingsView).toContain('DEVELOPER_MODE_STORAGE_KEY = "zc-developer-mode"');
    expect(settingsSurface).toContain('developerMode ? (');
    expect(settingsSurface).toContain('t("developerModeDesc")');
    expect(settingsView).toContain("setTimeout");
  });

  it("presents Quick Preview as a quiet non-interactive capability status", () => {
    const generalSettings = read("src/views/settings/sections/GeneralSettingsSection.tsx");
    const shellV26 = read("src/styles/w6-07-shell-v26.css");
    const zh = makeTranslator("zh");
    const en = makeTranslator("en");

    expect(zh("quickPreviewSettingStatus")).toBe("状态：已启用");
    expect(en("quickPreviewSettingStatus")).toBe("Status: Enabled");
    expect(generalSettings).toContain("data-settings-capability-status");
    expect(generalSettings).toContain('t("quickPreviewSettingStatus")');
    expect(generalSettings).not.toContain("SettingsSwitchControl");
    expect(generalSettings).not.toContain("data-settings-readonly-switch");
    expect(generalSettings).not.toContain('role="switch"');
    expect(shellV26).not.toContain("data-settings-readonly-switch");
  });

  it.each([
    { width: 1282, layout: "two-column", nav: "vertical", rows: "two-column" },
    { width: 969, layout: "two-column", nav: "vertical", rows: "two-column" },
    { width: 840, layout: "single-column", nav: "horizontal-scroll", rows: "two-column" },
    { width: 760, layout: "single-column", nav: "horizontal-scroll", rows: "stacked" }
  ] as const)("keeps the retained Settings composition at $width px", ({ width, layout, nav, rows }) => {
    const settingsPrimitives = read("src/views/settings/components/SettingsPrimitives.tsx");
    const shellV26 = read("src/styles/w6-07-shell-v26.css");
    const settingsSource = `${settingsPrimitives}\n${shellV26}`;
    const desktopComposition = width > 840;
    const stackedRows = width <= 760;

    expect(settingsSource).not.toContain("1179px");
    expect(settingsSource).not.toContain("1180px");
    expect(shellV26).toContain("@media (max-width: 840px)");
    expect(shellV26).toContain("@media (max-width: 760px)");
    expect(desktopComposition).toBe(layout === "two-column");
    expect((width <= 840)).toBe(nav === "horizontal-scroll");
    expect(stackedRows).toBe(rows === "stacked");

    if (desktopComposition) {
      expect(settingsPrimitives).toContain("min-[841px]:grid-cols-[200px_minmax(0,1fr)]");
      expect(shellV26).toContain("[data-settings-layout-grid] {");
    } else {
      expect(shellV26).toContain("grid-template-columns: 1fr !important;");
      expect(shellV26).toContain("[data-settings-section-nav-shell] nav {");
      expect(shellV26).toContain("display: flex;");
    }

    if (stackedRows) {
      expect(shellV26).toContain("[data-settings-content] [data-settings-row] {");
      expect(shellV26).toContain("grid-template-columns: 1fr;");
    } else {
      expect(shellV26).toContain("grid-template-columns: minmax(0, 1fr) 206px;");
    }
  });

  it("keeps Settings and Quick Preview parity geometry bounded to the retained structural target", () => {
    const shellV26 = read("src/styles/w6-07-shell-v26.css");
    const previewStyles = read("src/views/fileLibrary/preview/zenFloatingQuickPreview.css");
    const settingsPrimitives = read("src/views/settings/components/SettingsPrimitives.tsx");

    expect(shellV26).toContain("grid-template-columns: 196px minmax(0, 1fr) !important");
    expect(shellV26).toContain("max-width: 800px !important");
    expect(shellV26).toContain("grid-template-columns: minmax(0, 1fr) 206px");
    expect(shellV26).toContain("[data-settings-content] [data-settings-select-control]");
    expect(shellV26).toContain("width: 176px");
    expect(shellV26).toContain("@media (max-width: 760px)");
    expect(settingsPrimitives).toContain("createPortal(");
    expect(settingsPrimitives).toContain("document.body");
    expect(previewStyles).toContain("grid-template-columns: 84px minmax(0, 1fr) 112px");
    expect(previewStyles).toContain("padding: 28px");
    expect(previewStyles).toContain("padding: 15px");
    expect(previewStyles).toContain("min-height: 29px");
    expect(previewStyles).toContain("padding-inline: 8px");
  });

  it("keeps AI settings fail-closed, visibly dirty, localized, and keyboard-selectable", () => {
    const settingsView = read("src/views/settings/SettingsView.tsx");
    const settingsPrimitives = read("src/views/settings/components/SettingsPrimitives.tsx");
    const settingsNavigation = read("src/views/settings/controllers/useSettingsNavigationController.ts");
    const en = makeTranslator("en");

    expect(settingsView).toContain("data-ai-save-bar");
    expect(settingsView).toContain("data-ai-runtime-mode={runtimeAIUserMode}");
    expect(settingsView).toContain("data-ai-draft-mode={draftAIUserMode}");
    expect(settingsView).toContain('data-ai-settings-state="error"');
    expect(settingsView).toContain("disabled={!aiSettingsDirty || isSavingAISettings || isTestingAIConnection}");
    expect(settingsView).toContain("aiSettingsSaveFailed");
    expect(settingsView).not.toContain("setAiSettings(previous)");
    expect(settingsView).toContain('layout="three-option-responsive"');
    expect(settingsView).toContain('controlWidth="wide"');
    expect(settingsView).toContain("data-ai-status-region");
    expect(settingsView).toContain("data-ai-advanced-connection-grid");
    expect(settingsView).not.toContain("settings-ai-advanced-provider");
    expect(settingsPrimitives).toContain('role="radiogroup"');
    expect(settingsPrimitives).toContain('role="radio"');
    expect(settingsPrimitives).toContain("aria-checked={selected}");
    expect(settingsView).toContain("AI_CLASSIFICATION_PRESET_IDS");
    expect(settingsView.match(/id="settings-ai-provider"/g)).toHaveLength(1);
    expect(settingsView).toContain("apiKey: settings.apiKey");
    expect(settingsView).not.toContain("batchSize: preset.providerKind");
    expect(settingsNavigation).toContain("scrollSettingsSectionIntoView(settingsScrollRef.current, targetId, {");
    expect(settingsNavigation).toContain('container.addEventListener("scroll", scheduleUpdate');
    expect(settingsView).toContain("developerMode ? (");
    expect(settingsView).toContain('t("aiAdvancedConnection")');
    expect(settingsView).toContain("SettingsSecretField");
    expect(settingsView).toContain("disabled={aiDependentControlsDisabled}");
    expect(settingsView).not.toContain("min-[720px]:grid-cols");
    expect(settingsView).not.toContain("aiModeForSettings");
    expect(en("languageDesc")).not.toMatch(/[\u3400-\u9fff]/);
    expect(en("globalSearch")).toContain("folders");
    expect(en("aiChatPathLabel")).toBe("Chat endpoint path");
    expect(en("aiBatchSizeLabel")).toBe("Batch size");
    expect(en("aiTimeoutLabel")).toBe("Timeout (seconds)");
    expect(en("showApiKey")).toBe("Show API key");
    expect(en("hideApiKey")).toBe("Hide API key");
  });
});
