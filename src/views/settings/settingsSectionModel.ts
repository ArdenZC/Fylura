export const SETTINGS_SECTION_IDS = [
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
] as const;

export type SettingsSectionId = typeof SETTINGS_SECTION_IDS[number];

export const SETTINGS_NAV_SECTION_IDS = [
  "settings-general",
  "settings-file-management",
  "settings-search",
  "settings-ai-automation",
  "settings-advanced",
  "settings-about"
] as const;

export type SettingsNavigationSectionId = typeof SETTINGS_NAV_SECTION_IDS[number];

export const SETTINGS_NAV_SECTION_CONTENT_IDS = {
  "settings-general": ["settings-general"],
  "settings-file-management": ["settings-files-scan", "settings-managed-scopes"],
  "settings-search": ["settings-search"],
  "settings-ai-automation": ["settings-automation", "settings-ai"],
  "settings-advanced": ["settings-global-index", "settings-platform-diagnostics", "settings-privacy"],
  "settings-about": ["settings-about"]
} as const satisfies Record<SettingsNavigationSectionId, readonly SettingsSectionId[]>;

export const PROGRESSIVE_SETTINGS_SECTION_IDS = [
  "settings-global-index",
  "settings-platform-diagnostics",
  "settings-managed-scopes"
] as const satisfies readonly SettingsSectionId[];

const progressiveSectionIds = new Set<string>(PROGRESSIVE_SETTINGS_SECTION_IDS);

export function isProgressiveSettingsSectionId(sectionId: string) {
  return progressiveSectionIds.has(sectionId);
}

export function settingsSectionRequestTarget(sectionId: string) {
  if (sectionId === "settings-search-scope") return "settings-search";
  const navigationContent = SETTINGS_NAV_SECTION_CONTENT_IDS[sectionId as SettingsNavigationSectionId];
  return navigationContent?.[0] ?? sectionId;
}

export function settingsNavigationSectionId(sectionId: string) {
  const target = sectionId === "settings-log-retention"
    ? "settings-privacy"
    : settingsSectionRequestTarget(sectionId);
  const contentId = [...SETTINGS_SECTION_IDS]
    .sort((left, right) => right.length - left.length)
    .find((id) => target === id || target.startsWith(`${id}-`));
  if (!contentId) return target;
  const navigationGroup = Object.entries(SETTINGS_NAV_SECTION_CONTENT_IDS)
    .find(([, contentIds]) => contentIds.some((id) => id === contentId));
  return navigationGroup?.[0] ?? contentId;
}
