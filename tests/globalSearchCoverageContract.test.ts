import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { globalSearchFeedback } from "../src/components/spotlight/globalSearchFeedback";
import type {
  GlobalSearchResponse,
  GlobalSearchSourceHealth,
} from "../src/types/domain";

// Review #5678861648 defect 1 was a contract break between the Rust coverage
// snapshot and the Spotlight card: the backend could report
// `collection_complete = true` while an enabled source was not `ready`, and the
// card then rendered a normal "no results" message. The two layers are tested
// separately elsewhere; this test pins the wire fields and the status
// vocabulary so the pair cannot drift apart silently.

const modelsSource = readFileSync(
  resolve("src-tauri/src/global_index/models.rs"),
  "utf8",
);
const domainSource = readFileSync(resolve("src/types/domain.ts"), "utf8");
const dictionarySource = readFileSync(resolve("src/i18n/dictionary.ts"), "utf8");

function toCamelCase(value: string): string {
  return value.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
}

function rustStructFields(name: string): string[] {
  const body =
    new RegExp(`pub struct ${name} \\{([\\s\\S]*?)\\n\\}`).exec(modelsSource)?.[1] ??
    "";
  return [...body.matchAll(/pub (\w+):/g)]
    .map((match) => toCamelCase(match[1]))
    .sort();
}

function typescriptInterfaceFields(name: string): string[] {
  const body =
    new RegExp(`export interface ${name} \\{([\\s\\S]*?)\\n\\}`).exec(
      domainSource,
    )?.[1] ?? "";
  return [...body.matchAll(/^ {2}(\w+)\??:/gm)].map((match) => match[1]).sort();
}

const backendStatuses = [
  ...modelsSource.matchAll(/pub const INDEX_STATUS_[A-Z_]+: &str = "([a-z_]+)"/g),
].map((match) => match[1]);

const ready = "ready";

function response(
  status: string,
  collectionComplete: boolean,
  resultState: GlobalSearchResponse["resultState"],
): GlobalSearchResponse {
  const source: GlobalSearchSourceHealth = {
    sourceId: "gv_contract",
    enabled: true,
    provider: "windows_mft_usn",
    status,
    lastError: null,
    updatedAt: 0,
  };
  return {
    version: 2,
    requestId: "r1",
    normalizedQuery: "alpha",
    results: [],
    indexStatus: {
      platform: "windows",
      enabled: true,
      status,
      providerStatus: null,
      processedEntries: 0,
      collectionComplete,
      totalEntries: 0,
      indexedVolumes: 1,
      readyVolumes: collectionComplete ? 1 : 0,
      pendingVolumes: 0,
      lastSyncAt: null,
      lastError: null,
    },
    collectionComplete,
    resultState,
    sourceRevision: "rev",
    sourceHealth: [source],
  };
}

const t = (key: string) => key;

describe("global search coverage contract", () => {
  it("keeps the source-health wire shape identical on both sides", () => {
    expect(rustStructFields("GlobalSearchSourceHealth")).toEqual(
      typescriptInterfaceFields("GlobalSearchSourceHealth"),
    );
    expect(rustStructFields("GlobalSearchSourceHealth")).toContain("sourceId");
  });

  it("knows every backend index status", () => {
    // Guards the regex itself: a status rename must not silently shrink the
    // matrix below to an empty loop.
    expect(backendStatuses.length).toBeGreaterThanOrEqual(15);
    expect(backendStatuses).toContain(ready);
    expect(backendStatuses).toContain("permission_required");
    expect(backendStatuses).toContain("watcher_unavailable");
  });

  it("never claims an empty result for any non-ready backend status", () => {
    const degraded = backendStatuses.filter((status) => status !== ready);
    expect(degraded.length).toBeGreaterThan(0);
    for (const status of degraded) {
      const feedback = globalSearchFeedback(
        response(status, false, "partial"),
        t,
      );
      expect(feedback.title, status).not.toBe("commandNoResultsTitle");
      // A card that renders an untranslated key is a missing i18n entry.
      expect(dictionarySource, status).toContain(`${feedback.title}:`);
    }
  });

  it("claims an empty result only for a ready, complete snapshot", () => {
    const feedback = globalSearchFeedback(response(ready, true, "empty"), t);
    expect(feedback.title).toBe("commandNoResultsTitle");
    expect(dictionarySource).toContain(`${feedback.title}:`);
  });
});
