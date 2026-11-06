export const WINDOWS_GLOBAL_SEARCH_FEATURES: readonly [
  "global-search-filename-relevance-candidate",
  "global-search-filename-relevance-windows-default-on",
];

export function windowsReleaseFeatures(platform: string): readonly string[];
