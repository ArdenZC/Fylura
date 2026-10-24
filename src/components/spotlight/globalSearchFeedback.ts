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
  if (response.resultState === "failed" || statuses.some((status) => ["unavailable", "permission_required", "error"].includes(status))) {
    return { title: t("globalIndexStatusUnavailable"), description: t("globalSearchUnavailableDesc") };
  }
  if (statuses.includes("discovered")) {
    return { title: t("globalSearchNotIndexedTitle"), description: t("globalSearchNotIndexedDesc") };
  }
  return { title: t("globalIndexStatusPartial"), description: t("globalSearchUnverifiedDesc") };
}
