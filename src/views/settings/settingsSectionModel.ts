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

export const PROGRESSIVE_SETTINGS_SECTION_IDS = [
  "settings-global-index",
  "settings-platform-diagnostics",
  "settings-managed-scopes"
] as const satisfies readonly SettingsSectionId[];

const progressiveSectionIds = new Set<string>(PROGRESSIVE_SETTINGS_SECTION_IDS);

export const SETTINGS_NAV_SECTION_IDS = SETTINGS_SECTION_IDS;

export function isProgressiveSettingsSectionId(sectionId: string) {
  return progressiveSectionIds.has(sectionId);
}

export function settingsSectionRequestTarget(sectionId: string) {
  return sectionId === "settings-search-scope" ? "settings-search" : sectionId;
}

export function settingsNavigationSectionId(sectionId: string) {
  const target = settingsSectionRequestTarget(sectionId);
  const match = [...SETTINGS_SECTION_IDS]
    .sort((left, right) => right.length - left.length)
    .find((id) => target === id || target.startsWith(`${id}-`));
  return match ?? target;
}
