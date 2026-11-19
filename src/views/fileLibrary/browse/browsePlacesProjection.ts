import type { KnownLocation, SavedPlace, VolumeAvailability } from "../../../api/fileManagementApi";
import type { Translator } from "../../../types/ui";

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

const SYSTEM_LABEL_KEYS = {
  home: "browsePlaceHome",
  desktop: "browsePlaceDesktop",
  downloads: "browsePlaceDownloads",
  documents: "browsePlaceDocuments"
} as const;

type SystemLocationId = keyof typeof SYSTEM_LABEL_KEYS;

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

function displayNameFromRoutingHint(routingHint: string) {
  const normalized = routingHint.replace(/[\\/]+$/, "");
  const segments = normalized.split(/[\\/]/).filter(Boolean);
  return segments.at(-1) ?? normalized;
}
