import type { GlobalIndexSource } from "../types/domain";
import { isFullVolumeAuthorizationTarget, isGlobalIndexSourceAuthorized } from "../utils/globalIndexSources";

export interface GlobalSearchConsentApi {
  authorizeGlobalIndexSource(sourceId: string, confirmed: boolean): Promise<void>;
  setGlobalIndexSourceEnabled(sourceId: string, enabled: boolean): Promise<void>;
  startGlobalIndex(): Promise<void>;
}

export interface GlobalSearchConsentResult {
  /** Sources that were enabled by this explicit consent. */
  enabled: string[];
  /** Per-source failures; a partial success stays a real, reported success. */
  failed: { sourceId: string; error: string }[];
  /** Indexing could not be started, but the source authorization was saved. */
  startError: string | null;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Applies an explicit, per-source Global Search consent.
 *
 * Only sources the caller selected are touched. A Windows fixed NTFS volume is
 * indexed as a whole volume, so it first receives the existing explicit
 * full-volume authorization; every other source is enabled directly. Sources
 * that are not selected are never enabled, disabled or rebuilt, and existing
 * authorizations are preserved. No folder is added to the Managed Library and
 * no Managed Scan is started here.
 */
export async function enableGlobalSearchForSources(
  sources: readonly GlobalIndexSource[],
  selectedIds: ReadonlySet<string>,
  api: GlobalSearchConsentApi
): Promise<GlobalSearchConsentResult> {
  const result: GlobalSearchConsentResult = { enabled: [], failed: [], startError: null };
  for (const source of sources) {
    const sourceId = source.volume.id;
    if (!selectedIds.has(sourceId)) continue;
    try {
      if (isFullVolumeAuthorizationTarget(source) && !isGlobalIndexSourceAuthorized(source)) {
        await api.authorizeGlobalIndexSource(sourceId, true);
      }
      await api.setGlobalIndexSourceEnabled(sourceId, true);
      result.enabled.push(sourceId);
    } catch (error) {
      result.failed.push({ sourceId, error: message(error) });
    }
  }
  if (result.enabled.length > 0) {
    try {
      await api.startGlobalIndex();
    } catch (error) {
      result.startError = message(error);
    }
  }
  return result;
}
