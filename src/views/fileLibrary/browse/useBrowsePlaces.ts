import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fileManagementApi, type KnownLocation, type SavedPlace, type VolumeAvailability } from "../../../api/fileManagementApi";
import type { Translator } from "../../../types/ui";
import { displayNameFromRoutingHint } from "./browseSourceOwner";
import { mountedLocations, networkAvailabilityFromBrowseError, useMountedLocations } from "../navigation/mountedLocations";

export type BrowsePlaceEntryKind = "system" | "favorite" | "recent";

export interface BrowsePlaceEntry {
  /** Stable React key. Saved places own a backend id; system places use their projection id. */
  readonly key: string;
  readonly kind: BrowsePlaceEntryKind;
  readonly label: string;
  readonly path: string;
  readonly openable: boolean;
  readonly network: boolean;
  readonly availability?: VolumeAvailability;
  readonly locationId?: string;
}

export interface BrowsePlaceOpenResult {
  opened: boolean;
  sessionId?: string;
  error?: string;
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
        mountedLocations.recordNetworkBrowse(entry.locationId, entry.path, availability, typeof result === "boolean" ? undefined : result.sessionId);
        setEntries((current) => current.map((item) => item.key === entry.key ? { ...item, openable: true, availability } : item));
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

export function projectKnownLocations(locations: readonly KnownLocation[], t: Translator): BrowsePlaceEntry[] {
  return locations.map((location) => {
    const network = location.volume?.kind === "network";
    return {
      key: `system:${location.id}`,
      kind: "system" as const,
      label: systemLabel(location, t),
      path: location.path,
      openable: location.available || network,
      network,
      availability: network ? location.volume?.availability ?? "unknown" : undefined,
      locationId: network ? location.id : undefined
    };
  });
}

export function projectSavedPlaces(places: readonly SavedPlace[], t: Translator): BrowsePlaceEntry[] {
  return places.map((place) => ({
    key: `saved:${place.id}`,
    kind: place.favorite ? "favorite" as const : "recent" as const,
    label: place.name || displayNameFromRoutingHint(place.routingHint) || t("browsePlaceRecentFallback"),
    path: place.routingHint,
    openable: !place.unavailable,
    network: false
  }));
}

export function browsePlaceStatusLabel(entry: BrowsePlaceEntry, t: Translator): string {
  if (!entry.network) return t(entry.openable ? "browsePlaceAvailable" : "browsePlaceUnavailable");
  switch (entry.availability) {
    case "connected_unverified": return t("filesLocationConnectedUnverified");
    case "available": return t("filesLocationOpenVerified");
    case "permission_required": return t("filesLocationPermission");
    case "offline": return t("filesLocationOffline");
    default: return t("filesLocationUnknown");
  }
}

function systemLabel(location: KnownLocation, t: Translator): string {
  const key = SYSTEM_LABEL_KEYS[location.id as SystemLocationId];
  if (key) return t(key);
  return location.displayName || location.path;
}
