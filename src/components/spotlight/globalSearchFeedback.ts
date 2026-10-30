import type { GlobalSearchResponse } from "../../types/domain";
import type { Translator } from "../../types/ui";

/** Presentation of the backend snapshot; this never grants index coverage. */
export function globalSearchFeedback(response: GlobalSearchResponse, t: Translator) {
  if (response.resultState === "no_source") {
    return { title: t("globalSearchNoSourcesTitle"), description: t("globalSearchNoSourcesDesc") };
  }
  if (response.collectionComplete && response.resultState === "empty") {
    return { title: t("commandNoResultsTitle"), description: t("globalSearchReadyEmptyDesc") };
  }
  const enabled = response.sourceHealth.filter((source) => source.enabled);
  const statuses = enabled.map((source) => source.status);
  if (response.indexStatus.status === "paused" || (statuses.length > 0 && statuses.every((status) => status === "paused"))) {
    return { title: t("globalIndexStatusPaused"), description: t("globalSearchPausedDesc") };
  }
  if (statuses.some((status) => status === "indexing" || status === "syncing")) {
    return { title: t("globalIndexStatusIndexing"), description: t("globalSearchIndexingDesc") };
  }
  if (statuses.includes("rebuild_required")) {
    return { title: t("globalIndexStatusRebuildRequired"), description: t("globalSearchRebuildDesc") };
  }
  if (response.indexStatus.status === "database_busy" || statuses.includes("database_busy")) {
    return { title: t("globalIndexStatusDatabaseBusy"), description: t("globalSearchDatabaseBusyDesc") };
  }
  if (response.indexStatus.status === "service_busy" || statuses.includes("service_busy")) {
    return { title: t("globalIndexStatusServiceBusy"), description: t("globalSearchServiceBusyDesc") };
  }
  if (response.indexStatus.status === "permission_required" || statuses.includes("permission_required")) {
    return { title: t("globalIndexStatusPermissionRequired"), description: t("globalSearchPermissionRequiredDesc") };
  }
  // The baseline index completed but incremental change watching failed, so
  // results are real yet frozen. Surface that honestly instead of falling
  // through to a generic "partial" message.
  if (response.indexStatus.status === "watcher_unavailable" || statuses.includes("watcher_unavailable")) {
    return { title: t("globalIndexStatusWatcherUnavailable"), description: t("globalSearchWatcherUnavailableDesc") };
  }
  if (response.indexStatus.status === "error" || statuses.includes("error")) {
    return { title: t("globalIndexStatusError"), description: t("globalSearchUnavailableDesc") };
  }
  if (response.resultState === "failed" || statuses.includes("unavailable")) {
    return { title: t("globalIndexStatusUnavailable"), description: t("globalSearchUnavailableDesc") };
  }
  if (statuses.includes("discovered")) {
    return { title: t("globalSearchNotIndexedTitle"), description: t("globalSearchNotIndexedDesc") };
  }
  return { title: t("globalIndexStatusPartial"), description: t("globalSearchUnverifiedDesc") };
}
