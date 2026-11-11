import { useSyncExternalStore } from "react";
import { fileManagementApi, type KnownLocation, type VolumeAvailability } from "../../../api/fileManagementApi";
import { useFileManagementStore } from "../management/fileManagementStore";

interface NetworkAccessObservation {
  locationId: string;
  path: string;
  availability: VolumeAvailability;
  revision: number;
}

interface NetworkBrowseSession {
  locationId: string;
  path: string;
}

const MAX_NETWORK_OBSERVATIONS = 48;
const MAX_NETWORK_BROWSE_SESSIONS = 96;

export function networkAvailabilityFromBrowseError(error: unknown): VolumeAvailability {
  const message = typeof error === "string" ? error : String(error ?? "");
  if (/permission_denied|authentication_required|logon_failure|invalid_password/i.test(message)) return "permission_required";
  if (/directory_offline|directory_not_found|device_disconnected|network_unreachable|network_name_deleted/i.test(message)) return "offline";
  return "unknown";
}

interface MountedLocationsSnapshot {
  locations: KnownLocation[];
  loading: boolean;
  failed: boolean;
  ejectingId: string | null;
  ejectError: string | null;
}

/** One foreground, read-only discovery projection shared by all visible panes. */
export class MountedLocationProjection {
  private snapshot: MountedLocationsSnapshot = { locations: [], loading: false, failed: false, ejectingId: null, ejectError: null };
  private listeners = new Set<() => void>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private inFlight: Promise<void> | null = null;
  private networkObservations = new Map<string, NetworkAccessObservation>();
  private networkBrowseSessions = new Map<string, NetworkBrowseSession>();
  private latestNetworkBrowseSessionByLocation = new Map<string, string>();
  private networkObservationRevision = 0;
  constructor(private load = () => fileManagementApi.fileLibraryKnownLocations()) {}
  getSnapshot = () => this.snapshot;
  private publish(next: MountedLocationsSnapshot) {
    this.snapshot = next;
    this.listeners.forEach((listener) => listener());
  }
  refresh = (resetNetworkStatus = false) => {
    if (resetNetworkStatus) {
      this.networkObservations.clear();
      this.networkBrowseSessions.clear();
      this.latestNetworkBrowseSessionByLocation.clear();
    }
    if (this.inFlight) return this.inFlight;
    if (this.snapshot.ejectingId) return Promise.resolve();
    const requestObservationRevision = this.networkObservationRevision;
    this.publish({ ...this.snapshot, loading: true });
    this.inFlight = this.load().then((locations) => {
      const discovered = locations.filter((location) => location.kind === "volume" || location.kind === "cloud_provider");
      const live = discovered.map((location) => this.applyNetworkObservation(location, requestObservationRevision));
      const present = new Set(live.map((location) => location.id));
      // Retain bounded disconnected entries so a disappearing device does not
      // look like an empty list or silently become a different device.
      const missing = this.snapshot.locations.filter((location) => !present.has(location.id)).map((location) => {
        const observed = this.networkObservations.get(location.id);
        if (location.volume?.kind === "network"
          && observed?.path === location.path
          && observed.revision > requestObservationRevision) {
          return projectNetworkAvailability(location, observed.availability);
        }
        this.networkObservations.delete(location.id);
        if (location.volume?.kind === "network") {
          this.latestNetworkBrowseSessionByLocation.delete(location.id);
          return { ...location, available: false, volume: { ...location.volume, availability: "offline" as const } };
        }
        return { ...location, available: false };
      });
      this.publish({ ...this.snapshot, locations: [...live, ...missing].slice(0, 48), loading: false, failed: false });
    }).catch(() => {
      this.publish({ ...this.snapshot, locations: this.snapshot.locations.map((location) => {
        if (location.volume?.kind !== "network") return { ...location, available: false };
        const observed = this.networkObservations.get(location.id);
        if (observed?.path === location.path && observed.revision > requestObservationRevision) {
          return projectNetworkAvailability(location, observed.availability);
        }
        return { ...location, available: false, volume: { ...location.volume, availability: "unknown" as const } };
      }), loading: false, failed: true });
    }).finally(() => { this.inFlight = null; });
    return this.inFlight;
  };
  recordNetworkBrowse = (
    locationId: string,
    path: string,
    availability: VolumeAvailability,
    sessionId?: string
  ) => {
    const current = this.snapshot.locations.find((location) => location.id === locationId && location.path === path);
    if (current && current.volume?.kind !== "network") return;
    this.networkObservations.delete(locationId);
    this.networkObservations.set(locationId, {
      locationId,
      path,
      availability,
      revision: ++this.networkObservationRevision
    });
    while (this.networkObservations.size > MAX_NETWORK_OBSERVATIONS) {
      const oldest = this.networkObservations.keys().next().value;
      if (oldest === undefined) break;
      this.networkObservations.delete(oldest);
    }
    if (sessionId) {
      this.networkBrowseSessions.delete(sessionId);
      this.networkBrowseSessions.set(sessionId, { locationId, path });
      this.latestNetworkBrowseSessionByLocation.set(locationId, sessionId);
      while (this.networkBrowseSessions.size > MAX_NETWORK_BROWSE_SESSIONS) {
        const oldest = this.networkBrowseSessions.keys().next().value;
        if (oldest === undefined) break;
        this.networkBrowseSessions.delete(oldest);
      }
    } else {
      // An open attempt that fails before creating a Browse session is newer
      // than any outstanding enumeration, so an older pane must not overwrite
      // this explicit failure after it returns.
      this.latestNetworkBrowseSessionByLocation.delete(locationId);
    }
    this.publish({ ...this.snapshot, locations: this.snapshot.locations.map((location) =>
      location.id === locationId && location.path === path && location.volume?.kind === "network"
        ? projectNetworkAvailability(location, availability)
        : location) });
  };
  recordNetworkBrowseSessionResult = (sessionId: string, availability: VolumeAvailability) => {
    const session = this.networkBrowseSessions.get(sessionId);
    if (!session || this.latestNetworkBrowseSessionByLocation.get(session.locationId) !== sessionId) return;
    this.recordNetworkBrowse(session.locationId, session.path, availability);
    // Keep this session current for a later root refresh in the same pane.
    this.latestNetworkBrowseSessionByLocation.set(session.locationId, sessionId);
  };
  private applyNetworkObservation(location: KnownLocation, requestObservationRevision: number) {
    if (location.volume?.kind !== "network") return location;
    const observed = this.networkObservations.get(location.id);
    if (!observed || observed.path !== location.path) return location;
    if (observed.revision > requestObservationRevision) {
      return projectNetworkAvailability(location, observed.availability);
    }
    // A fresh OS connection record may be offline or permission-denied. A
    // prior explicit Browse result is retained only while the provider still
    // reports a connection record; manual refresh clears it before this merge.
    if (location.volume.availability === "offline" || location.volume.availability === "permission_required") {
      this.networkObservations.delete(location.id);
      this.latestNetworkBrowseSessionByLocation.delete(location.id);
      return location;
    }
    return projectNetworkAvailability(location, observed.availability);
  }
  eject = async (location: KnownLocation) => {
    if (this.snapshot.ejectingId || !location.available || !location.volume?.canEject || !location.volume.identity) return;
    this.publish({ ...this.snapshot, ejectingId: location.id, ejectError: null });
    try {
      await this.inFlight;
      await fileManagementApi.ejectVolume({ locationId: location.id, identity: location.volume.identity });
      this.publish({ ...this.snapshot, locations: this.snapshot.locations.map((item) => item.id === location.id ? { ...item, available: false } : item) });
    } catch (error) {
      this.publish({ ...this.snapshot, ejectError: String(error) });
    } finally {
      this.publish({ ...this.snapshot, ejectingId: null });
      const root = location.path.replace(/\\/g, "/").replace(/\/+$/, "");
      await Promise.allSettled(Object.values(useFileManagementStore.getState().surfaces).map((owner) => {
        const surface = owner.read();
        const path = surface?.routingHint?.replace(/\\/g, "/").replace(/\/+$/, "");
        const key = root.includes(":") ? root.toLowerCase() : root;
        const candidate = root.includes(":") ? path?.toLowerCase() : path;
        return candidate && (candidate === key || candidate.startsWith(`${key}/`)) ? surface?.refresh() : undefined;
      }));
      await this.refresh();
    }
  };
  private foregroundRefresh = () => {
    if (document.visibilityState !== "hidden") void this.refresh();
  };
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    if (this.listeners.size === 1) {
      this.foregroundRefresh();
      this.timer = setInterval(this.foregroundRefresh, 5000);
      window.addEventListener("focus", this.foregroundRefresh);
      document.addEventListener("visibilitychange", this.foregroundRefresh);
    }
    return () => {
      this.listeners.delete(listener);
      if (!this.listeners.size) {
        if (this.timer) clearInterval(this.timer);
        this.timer = null;
        window.removeEventListener("focus", this.foregroundRefresh);
        document.removeEventListener("visibilitychange", this.foregroundRefresh);
      }
    };
  };
}

function projectNetworkAvailability(location: KnownLocation, availability: VolumeAvailability): KnownLocation {
  return {
    ...location,
    available: availability === "available",
    volume: location.volume ? { ...location.volume, availability } : location.volume
  };
}

export const mountedLocations = new MountedLocationProjection();
const inactiveSubscription = () => () => undefined;
export function useMountedLocations(enabled: boolean) {
  const snapshot = useSyncExternalStore(enabled ? mountedLocations.subscribe : inactiveSubscription, mountedLocations.getSnapshot, mountedLocations.getSnapshot);
  return {
    ...snapshot,
    refresh: mountedLocations.refresh,
    recordNetworkBrowse: mountedLocations.recordNetworkBrowse,
    eject: mountedLocations.eject
  };
}
