import type {
  FileLibrarySummary,
  FileQueryResponseV2,
  FileQuerySpecV2
} from "../../../types/domain";
import type {
  BrowseEntry,
  BrowsePage
} from "../../../types/fileWorkspace";
import type {
  BrowsePresentationCollectionContext,
  BrowsePresentationEntry,
  LibraryPresentationCollectionContext,
  LibraryPresentationEntry,
  PresentationEntry,
  PresentationRenderKey
} from "./contracts";

type LibraryCollectionResponse = Pick<
  FileQueryResponseV2,
  "queryFingerprint" | "snapshotRevision"
>;

/**
 * Adapt a managed summary into rendering facts only. Query/selection
 * provenance belongs to adaptLibraryCollection, not to each row.
 */
export function adaptLibrarySummary(summary: FileLibrarySummary): LibraryPresentationEntry {
  const extension = nonEmpty(summary.extension);
  const typeHint = nonEmpty(summary.fileType);
  const materialization = materializationFromAvailability(summary.nativeSemantics?.contentAvailability);

  return {
    source: "library",
    renderKey: renderKey(["library", summary.id]),
    entryRef: { kind: "managed", fileId: summary.id },
    availability: summary.isStale ? "missing" : "available",
    displayName: summary.name,
    entryKind: summary.isDirectory ? "directory" : "file",
    ...(extension === undefined ? {} : { extension }),
    ...(typeHint === undefined ? {} : { typeHint }),
    size: summary.size,
    modifiedAt: summary.modifiedAt,
    createdAt: summary.createdAt,
    ...(materialization === undefined ? {} : { materialization })
  };
}

/**
 * Adapt one live Browse entry without turning displayPath or any render key
 * into an authority-bearing value.
 */
export function adaptBrowseEntry(entry: BrowseEntry): BrowsePresentationEntry {
  const extension = nonEmpty(entry.extension);

  return {
    source: "browse",
    renderKey: renderKey(["browse", entry.ref.browseSessionId, entry.ref.entryId]),
    entryRef: entry.ref,
    displayPath: entry.displayPath,
    ...(entry.pathRef === undefined ? {} : { pathRef: entry.pathRef }),
    displayName: entry.name,
    entryKind: entry.kind,
    ...(extension === undefined ? {} : { extension }),
    ...(entry.size === undefined ? {} : { size: entry.size }),
    ...(entry.modifiedAt === undefined ? {} : { modifiedAt: entry.modifiedAt }),
    ...(entry.createdAt === undefined ? {} : { createdAt: entry.createdAt }),
    ...(entry.isHidden === undefined ? {} : { isHidden: entry.isHidden }),
    materialization: entry.materialization
  };
}

/**
 * Preserve the current Query V2 collection clock once at collection scope.
 * The optional query is cloned because the presentation snapshot must not
 * retain a mutable source object or create another query authority.
 */
export function adaptLibraryCollection(
  response: LibraryCollectionResponse,
  query?: FileQuerySpecV2
): LibraryPresentationCollectionContext {
  return {
    source: "library",
    provenance: {
      queryFingerprint: response.queryFingerprint,
      snapshotRevision: response.snapshotRevision,
      ...(query === undefined ? {} : { query: cloneQuerySpec(query) })
    }
  };
}

/**
 * Preserve Browse publication identity and source-declared completeness. A
 * partial page never gains a known count merely because it contains rows.
 */
export function adaptBrowsePageCollection(page: BrowsePage): BrowsePresentationCollectionContext {
  const knownCount = page.completion === "complete" ? page.knownCount : undefined;

  return {
    source: "browse",
    provenance: {
      sessionId: page.sessionId,
      requestId: page.requestId,
      enumerationId: page.enumerationId,
      completion: page.completion,
      ...(knownCount === undefined ? {} : { knownCount })
    }
  };
}

function renderKey(parts: readonly string[]): PresentationRenderKey {
  // A JSON tuple gives the source tag and each opaque component structural
  // boundaries. There is intentionally no inverse parser or resolver.
  return JSON.stringify(["presentation-entry-v1", ...parts]) as PresentationRenderKey;
}

function nonEmpty(value: string | undefined): string | undefined {
  return value === undefined || value.length === 0 ? undefined : value;
}

function materializationFromAvailability(value: string | undefined) {
  if (value === undefined) return undefined;
  switch (value) {
    case "local":
      return "local" as const;
    case "not_local":
      return "remote_placeholder" as const;
    case "downloading":
      return "hydrating" as const;
    case "metadata_only":
      return "metadata_only" as const;
    case "unavailable":
      return "unavailable" as const;
    case "unknown":
    default:
      return "unknown" as const;
  }
}

function cloneQuerySpec(query: FileQuerySpecV2): FileQuerySpecV2 {
  const scope = query.scope.kind === "all_enabled_roots"
    ? { kind: "all_enabled_roots" as const }
    : query.scope.kind === "roots"
      ? { kind: "roots" as const, scanRootIds: [...query.scope.scanRootIds] }
      : { kind: "current_scan" as const, scanSessionId: query.scope.scanSessionId };

  return {
    scope,
    text: query.text,
    filters: {
      ...query.filters,
      fileTypes: [...query.filters.fileTypes],
      purposes: [...query.filters.purposes],
      lifecycles: [...query.filters.lifecycles],
      risks: [...query.filters.risks],
      tagsAllOf: [...query.filters.tagsAllOf],
      tagsAnyOf: [...query.filters.tagsAnyOf],
      tagsNoneOf: [...query.filters.tagsNoneOf]
    },
    sort: { ...query.sort }
  };
}

/** A typed helper for source owners rendering only a supplied window. */
export function adaptPresentationWindow(
  entries: readonly (FileLibrarySummary | BrowseEntry)[]
): PresentationEntry[] {
  return entries.map((entry) => "ref" in entry
    ? adaptBrowseEntry(entry)
    : adaptLibrarySummary(entry));
}
