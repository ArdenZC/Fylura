import { useSyncExternalStore } from "react";
import { fileManagementApi, type KnownLocation } from "../../../api/fileManagementApi";

interface MountedLocationsSnapshot {
  locations: KnownLocation[];
  loading: boolean;
  failed: boolean;
}

/** One foreground, read-only discovery projection shared by all visible panes. */
export class MountedLocationProjection {
  private snapshot: MountedLocationsSnapshot = { locations: [], loading: false, failed: false };
  private listeners = new Set<() => void>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private inFlight: Promise<void> | null = null;
  constructor(private load = () => fileManagementApi.fileLibraryKnownLocations()) {}
  getSnapshot = () => this.snapshot;
  private publish(next: MountedLocationsSnapshot) {
    this.snapshot = next;
    this.listeners.forEach((listener) => listener());
  }
  refresh = () => {
    if (this.inFlight) return this.inFlight;
    this.publish({ ...this.snapshot, loading: true });
    this.inFlight = this.load().then((locations) => {
      this.publish({ locations: locations.filter((location) => location.kind === "volume"), loading: false, failed: false });
    }).catch(() => {
      this.publish({ locations: this.snapshot.locations.map((location) => ({ ...location, available: false })), loading: false, failed: true });
    }).finally(() => { this.inFlight = null; });
    return this.inFlight;
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

const mountedLocations = new MountedLocationProjection();
const inactiveSubscription = () => () => undefined;
export function useMountedLocations(enabled: boolean) {
  const snapshot = useSyncExternalStore(enabled ? mountedLocations.subscribe : inactiveSubscription, mountedLocations.getSnapshot);
  return { ...snapshot, refresh: mountedLocations.refresh };
}
