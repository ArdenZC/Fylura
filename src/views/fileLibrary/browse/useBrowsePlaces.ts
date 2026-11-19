import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fileManagementApi, type KnownLocation } from "../../../api/fileManagementApi";
import type { Translator } from "../../../types/ui";
import { mountedLocations, networkAvailabilityFromBrowseError, useMountedLocations } from "../navigation/mountedLocations";
import { browsePlaceStatusLabel, projectKnownLocations, projectSavedPlaces, type BrowsePlaceEntry } from "./browsePlacesProjection";

export { browsePlaceStatusLabel, projectKnownLocations, projectSavedPlaces } from "./browsePlacesProjection";
export type { BrowsePlaceEntry, BrowsePlaceEntryKind } from "./browsePlacesProjection";

export interface BrowsePlaceOpenResult {
  opened: boolean;
  sessionId?: string;
  error?: string;
}
export type BrowsePlacesState = "idle" | "loading" | "ready" | "failed";

/**
 * Entry points shown before any folder is open. Two independent read-only
 * projections are combined:
 *
 * - `file_workspace_known_locations` (well-known folders and mounted volumes),
 * - `file_workspace_places` (existing favorites and recent Browse locations).
 *
 * Nothing here creates a Managed Library root, authorizes a Global Index
 * source, starts a scan or reads file content. Choosing an entry is handed to
 * the Browse source owner, which re-admits the path through the same
 * user-intent admission as the OS folder picker.
 */
export function useBrowsePlaces({ enabled, openPath, t }: {
  enabled: boolean;
  openPath: (path: string, displayName: string) => Promise<boolean | BrowsePlaceOpenResult>;
  t: Translator;
}) {
  const [entries, setEntries] = useState<BrowsePlaceEntry[]>([]);
  const mounted = useMountedLocations(enabled);
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
    setEntries([
      ...projectKnownLocations(known ?? [], t),
      ...projectSavedPlaces(saved ?? [], t)
    ]);
    setState(known === null && saved === null ? "failed" : "ready");
  }, [t]);

  useEffect(() => {
    if (!enabled) return undefined;
    void load();
    return () => { requestEpoch.current += 1; };
  }, [enabled, load]);

  const reload = useCallback(async () => {
    await Promise.all([load(), mounted.refresh(true)]);
  }, [load, mounted.refresh]);

  const openPlace = useCallback(async (entry: BrowsePlaceEntry) => {
    if (openingKey !== null || !entry.openable) return false;
    setOpeningKey(entry.key);
    setOpenFailed(false);
    try {
      const result = await openPath(entry.path, entry.label);
      const opened = typeof result === "boolean" ? result : result.opened;
      if (entry.network && entry.locationId) {
        const availability = opened
          ? "connected_unverified"
          : networkAvailabilityFromBrowseError(typeof result === "boolean" ? "" : result.error);
        if (availability !== null) {
          mountedLocations.recordNetworkBrowse(entry.locationId, entry.path, availability, typeof result === "boolean" ? undefined : result.sessionId);
          setEntries((current) => current.map((item) => item.key === entry.key ? { ...item, openable: true, availability } : item));
        }
      }
      if (!opened) setOpenFailed(true);
      return opened;
    } finally {
      setOpeningKey(null);
    }
  }, [openPath, openingKey]);

  const projectedEntries = useMemo(
    () => projectMountedNetworkStatuses(entries, mounted.locations),
    [entries, mounted.locations]
  );

  return {
    entries: projectedEntries,
    state,
    openingKey,
    openFailed,
    reload,
    openPlace
  };
}

export function projectMountedNetworkStatuses(
  entries: readonly BrowsePlaceEntry[],
  locations: readonly KnownLocation[]
): BrowsePlaceEntry[] {
  const mountedByKey = new Map(locations
    .filter((location) => location.volume?.kind === "network")
    .map((location) => [`${location.id}\0${location.path}`, location] as const));
  return entries.map((entry) => {
    if (!entry.network || !entry.locationId) return entry;
    const mounted = mountedByKey.get(`${entry.locationId}\0${entry.path}`);
    if (!mounted?.volume || mounted.volume.kind !== "network") return entry;
    return { ...entry, availability: mounted.volume.availability };
  });
}
