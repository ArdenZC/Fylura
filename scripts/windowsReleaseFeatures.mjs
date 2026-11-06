export const WINDOWS_GLOBAL_SEARCH_FEATURES = Object.freeze([
  "global-search-filename-relevance-candidate",
  "global-search-filename-relevance-windows-default-on",
]);

export function windowsReleaseFeatures(platform) {
  const features = ["desktop-runtime", "global-search-filename-relevance-candidate"];
  if (platform === "win32") {
    features.push("global-search-filename-relevance-windows-default-on");
  }
  return Object.freeze(features);
}
