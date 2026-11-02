import { useCallback, useEffect, useRef, useState } from "react";
import { fileManagementApi, type KnownLocation, type SavedPlace } from "../../../api/fileManagementApi";
import { useCommandContext, useI18nContext } from "../../../contexts/AppContexts";
import type { Translator } from "../../../types/ui";
import { useFileLibraryExperience } from "../FileLibraryExperienceProvider";
import { displayNameFromRoutingHint, workspacePlatformForRuntime } from "./browseSourceOwner";

export type BrowsePlaceEntryKind = "system" | "favorite" | "recent";

export interface BrowsePlaceEntry {
  /** Stable React key. Saved places own a backend id; system places use their projection id. */
  readonly key: string;
  readonly kind: BrowsePlaceEntryKind;
  readonly label: string;
  readonly path: string;
  readonly available: boolean;
}

export type BrowsePlacesState = "idle" | "loading" | "ready" | "failed";

const SYSTEM_LABEL_KEYS = {
  home: "browsePlaceHome",
  desktop: "browsePlaceDesktop",
  downloads: "browsePlaceDownloads",
  documents: "browsePlaceDocuments"
} as const;

type SystemLocationId = keyof typeof SYSTEM_LABEL_KEYS;

/**
 * Entry points shown before any folder is open. Two independent read-only
 * projections are combined:
 *
 * - `file_workspace_known_locations` (well-known folders and mounted volumes),
 * - `file_workspace_places` (existing favorites and recent Browse locations).
 *
 * Nothing here creates a Managed Library root, authorizes a Global Index
 * source, starts a scan or reads file content. Choosing an entry re-admits the
 * path through the existing user-intent Browse admission, exactly like the
 * folder picker.
 */
export function useBrowsePlaces({ enabled }: { enabled: boolean }) {
  const { controller } = useFileLibraryExperience();
  const { t } = useI18nContext();
  const { platform } = useCommandContext();
  const [entries, setEntries] = useState<BrowsePlaceEntry[]>([]);
  const [state, setState] = useState<BrowsePlacesState>("idle");
  const [openingKey, setOpeningKey] = useState<string | null>(null);
  const [openFailed, setOpenFailed] = useState(false);
  const requestEpoch = useRef(0);

  const load = useCallback(async () => {
    const epoch = ++requestEpoch.current;
    setState("loading");
    const [known, saved] = await Promise.all([
      fileManagementApi.fileLibraryKnownLocations().catch(() => null),
      fileManagementApi.fileLibraryPlaces().catch(() => null)
    ]);
    if (epoch !== requestEpoch.current) return;
    const next = [
      ...projectKnownLocations(known ?? [], t),
      ...projectSavedPlaces(saved ?? [], t)
    ];
    setEntries(next);
    setState(known === null && saved === null ? "failed" : "ready");
  }, [t]);

  useEffect(() => {
    if (!enabled) return;
    void load();
    return () => { requestEpoch.current += 1; };
  }, [enabled, load]);

  const openPlace = useCallback(async (entry: BrowsePlaceEntry) => {
    if (openingKey !== null || !entry.available) return false;
    const workspacePlatform = workspacePlatformForRuntime(platform);
    if (workspacePlatform === null) {
      setOpenFailed(true);
      return false;
    }
    setOpeningKey(entry.key);
    setOpenFailed(false);
    try {
      const response = await controller.openBrowse({
        platform: workspacePlatform,
        routingHint: entry.path,
        displayHint: entry.label
      });
      if (response === null) {
        setOpenFailed(true);
        return false;
      }
      return true;
    } catch {
      setOpenFailed(true);
      return false;
    } finally {
      setOpeningKey(null);
    }
  }, [controller, openingKey, platform]);

  return {
    entries,
    state,
    openingKey,
    openFailed,
    reload: load,
    openPlace,
    dismissOpenFailed: () => setOpenFailed(false)
  };
}

export function projectKnownLocations(locations: readonly KnownLocation[], t: Translator): BrowsePlaceEntry[] {
  return locations.map((location) => ({
    key: `system:${location.id}`,
    kind: "system" as const,
    label: systemLabel(location, t),
    path: location.path,
    available: location.available
  }));
}

export function projectSavedPlaces(places: readonly SavedPlace[], t: Translator): BrowsePlaceEntry[] {
  return places.map((place) => ({
    key: `saved:${place.id}`,
    kind: place.favorite ? "favorite" as const : "recent" as const,
    label: place.name || displayNameFromRoutingHint(place.routingHint) || t("browsePlaceRecentFallback"),
    path: place.routingHint,
    available: !place.unavailable
  }));
}

function systemLabel(location: KnownLocation, t: Translator): string {
  const key = SYSTEM_LABEL_KEYS[location.id as SystemLocationId];
  if (key) return t(key);
  return location.displayName || location.path;
}
