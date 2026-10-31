import type { Translator, View } from "../../types/ui";

export type SpotlightCommandGroup = "actions" | "settings" | "history";
export type SpotlightCommandSurface = "main" | "standalone" | "browser";
type TranslationKey = Parameters<Translator>[0];

export type SpotlightCommandDefinition = {
  id: string;
  labelKey: TranslationKey;
  descriptionKey: TranslationKey;
  keywords: readonly string[];
  group: SpotlightCommandGroup;
  view: View;
  settingsSection?: string;
  defaultShortcutHint: string | null;
  availability: readonly SpotlightCommandSurface[];
};

export type SpotlightCommand = {
  kind: "command";
  id: string;
  label: string;
  description: string;
  keywords: string[];
  group: SpotlightCommandGroup;
  view: View;
  settingsSection?: string;
  defaultShortcutHint: string | null;
  enabled: boolean;
  disabledReason: string | null;
};

export const SPOTLIGHT_COMMAND_CATALOG: readonly SpotlightCommandDefinition[] = [
  definition("overview", "overview", "commandOverviewDesc", ["概览", "overview", "扫描", "scan"], "actions", "scanner"),
  definition("library", "filesWorkspace", "commandLibraryDesc", ["文件", "library", "files"], "actions", "library"),
  definition("suggestions", "organizeFiles", "commandOrganizeFilesDesc", ["整理", "建议", "organize", "suggestions", "files"], "actions", "organize"),
  definition("cleanup", "storageCleanup", "commandCleanupDesc", ["清理", "空间", "cleanup", "storage", "safe trash"], "actions", "cleanup"),
  definition("history", "history", "commandHistoryDesc", ["历史", "恢复", "history", "restore"], "history", "restore"),
  definition("automation", "automation", "commandAutomationDesc", ["自动化", "规则", "automation", "rules"], "actions", "automation"),
  definition("settings", "settings", "commandSettingsDesc", ["设置", "偏好", "settings"], "settings", "settings"),
  definition("search-scope-settings", "searchScopeSettings", "commandSearchScopeDesc", ["搜索范围", "范围设置", "search scope"], "settings", "settings", "all", "settings-search-scope"),
  definition("global-index-settings", "globalIndexSettings", "commandGlobalIndexDesc", ["全局索引", "managed", "global index", "index"], "settings", "settings", "all", "settings-global-index"),
  definition("theme-settings", "commandThemeSettings", "commandThemeDesc", ["主题", "外观", "深色", "浅色", "theme"], "settings", "settings", "all", "settings-general"),
  definition("ai-settings", "commandAISettings", "commandAIDesc", ["AI", "模型", "ollama", "cloud"], "settings", "settings", "all", "settings-ai")
] as const;

export function createCommandRegistry(
  t: Translator,
  surface: SpotlightCommandSurface = "main"
): SpotlightCommand[] {
  return SPOTLIGHT_COMMAND_CATALOG
    .map((item) => {
      const commandAvailability = resolveSpotlightCommandAvailability(item, surface);
      return {
      kind: "command",
      id: item.id,
      label: t(item.labelKey),
      description: t(item.descriptionKey),
      keywords: [...item.keywords],
      group: item.group,
      view: item.view,
      settingsSection: item.settingsSection,
      defaultShortcutHint: item.defaultShortcutHint,
      ...commandAvailability
    };
    });
}

export function resolveSpotlightCommandAvailability(
  definition: SpotlightCommandDefinition,
  surface: SpotlightCommandSurface
) {
  const enabled = definition.availability.includes(surface);
  return {
    enabled,
    disabledReason: enabled ? null : "command_surface_unavailable"
  };
}

export function queryCommandRegistry(query: string, registry: SpotlightCommand[]) {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [];
  return registry.filter((item) =>
    [item.label, item.description, ...item.keywords]
      .join(" ")
      .toLocaleLowerCase()
      .includes(needle)
  );
}

export function executeSpotlightCommand(
  command: SpotlightCommand,
  actions: {
    setView: (view: View) => void;
    requestSettingsSection: (sectionId: string) => void;
    onClose: () => void;
  }
) {
  actions.setView(command.view);
  if (command.settingsSection) actions.requestSettingsSection(command.settingsSection);
  actions.onClose();
}

export const SETTINGS_SECTION_EVENT = "fylura:settings-section";

export function requestSettingsSection(sectionId: string) {
  try {
    window.sessionStorage.setItem(SETTINGS_SECTION_EVENT, sectionId);
  } catch {
    // The in-memory event still handles the current window.
  }
  window.dispatchEvent(new CustomEvent(SETTINGS_SECTION_EVENT, { detail: sectionId }));
}

function definition(
  id: string,
  labelKey: TranslationKey,
  descriptionKey: TranslationKey,
  keywords: readonly string[],
  group: SpotlightCommandGroup,
  view: View,
  _availability: "all" = "all",
  settingsSection?: string
): SpotlightCommandDefinition {
  return {
    id,
    labelKey,
    descriptionKey,
    keywords,
    group,
    view,
    settingsSection,
    defaultShortcutHint: null,
    availability: ["main", "standalone", "browser"]
  };
}
