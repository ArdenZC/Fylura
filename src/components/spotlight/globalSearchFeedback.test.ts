import { describe, expect, it } from "vitest";
import { globalSearchFeedback } from "./globalSearchFeedback";
import type {
  GlobalSearchResponse,
  GlobalSearchSourceHealth,
} from "../../types/domain";

const t = (key: string) => key;

// Mirrors tauri GlobalSearchSourceHealth (source_id/provider/status/last_error/
// updated_at) so the fixture cannot drift into the source-list shape.
function sourceHealth(
  sourceId: string,
  status = "ready",
  enabled = true,
): GlobalSearchSourceHealth {
  return {
    sourceId,
    enabled,
    provider: "windows_mft_usn",
    status,
    lastError: null,
    updatedAt: 0,
  };
}

function response(
  overrides: Partial<GlobalSearchResponse>,
): GlobalSearchResponse {
  return {
    version: 2,
    requestId: "r1",
    normalizedQuery: "alpha",
    results: [],
    indexStatus: {
      platform: "windows",
      enabled: true,
      status: "ready",
      providerStatus: null,
      processedEntries: 0,
      collectionComplete: true,
      totalEntries: 0,
      indexedVolumes: 1,
      readyVolumes: 1,
      pendingVolumes: 0,
      lastSyncAt: null,
      lastError: null,
    },
    collectionComplete: true,
    resultState: "empty",
    sourceRevision: "rev",
    sourceHealth: [sourceHealth("gv_1")],
    ...overrides,
  } as GlobalSearchResponse;
}

describe("globalSearchFeedback coverage honesty", () => {
  it("reports a normal empty result only when coverage is complete", () => {
    const feedback = globalSearchFeedback(response({}), t);
    expect(feedback.title).toBe("commandNoResultsTitle");
  });

  it("never claims an empty result when a source needs permission", () => {
    // Review #5678861648 defect 1: this used to render the normal
    // "no results" card because collection_complete ignored the status.
    const feedback = globalSearchFeedback(
      response({
        collectionComplete: false,
        resultState: "failed",
        indexStatus: {
          ...response({}).indexStatus,
          status: "permission_required",
          collectionComplete: false,
        },
        sourceHealth: [sourceHealth("gv_1", "permission_required")],
      }),
      t,
    );
    expect(feedback.title).toBe("globalIndexStatusPermissionRequired");
    expect(feedback.title).not.toBe("commandNoResultsTitle");
  });

  it("explains frozen results when the change watcher is unavailable", () => {
    const feedback = globalSearchFeedback(
      response({
        collectionComplete: false,
        resultState: "partial",
        indexStatus: {
          ...response({}).indexStatus,
          status: "error",
          collectionComplete: false,
        },
        sourceHealth: [sourceHealth("gv_1", "watcher_unavailable")],
      }),
      t,
    );
    expect(feedback.title).toBe("globalIndexStatusWatcherUnavailable");
  });

  it("reports no_source when nothing is enabled", () => {
    const feedback = globalSearchFeedback(
      response({
        collectionComplete: false,
        resultState: "no_source",
        indexStatus: {
          ...response({}).indexStatus,
          enabled: false,
          status: "unavailable",
          indexedVolumes: 0,
          readyVolumes: 0,
          collectionComplete: false,
        },
        sourceHealth: [],
      }),
      t,
    );
    expect(feedback.title).toBe("globalSearchNoSourcesTitle");
  });

  it("reports partial when one of several sources is still indexing", () => {
    const feedback = globalSearchFeedback(
      response({
        results: [
          {
            id: "e1",
            volumeId: "gv_1",
            name: "alpha-report.txt",
            path: "Z:\\qa\\alpha-report.txt",
            extension: "txt",
            isDirectory: false,
            size: 29,
          },
        ] as GlobalSearchResponse["results"],
        collectionComplete: false,
        resultState: "partial",
        indexStatus: {
          ...response({}).indexStatus,
          status: "partial",
          indexedVolumes: 2,
          readyVolumes: 1,
          collectionComplete: false,
        },
        sourceHealth: [
          sourceHealth("gv_1", "ready"),
          sourceHealth("gv_2", "indexing"),
        ],
      }),
      t,
    );
    expect(feedback.title).toBe("globalIndexStatusIndexing");
  });
});
