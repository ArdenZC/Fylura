import { useSyncExternalStore } from "react";
import { fileManagementApi, type KnownLocation } from "../../../api/fileManagementApi";
import { useFileManagementStore } from "../management/fileManagementStore";

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
  constructor(private load = () => fileManagementApi.fileLibraryKnownLocations()) {}
  getSnapshot = () => this.snapshot;
  private publish(next: MountedLocationsSnapshot) {
    this.snapshot = next;
    this.listeners.forEach((listener) => listener());
  }
  refresh = () => {
    if (this.inFlight) return this.inFlight;
    if (this.snapshot.ejectingId) return Promise.resolve();
    this.publish({ ...this.snapshot, loading: true });
    this.inFlight = this.load().then((locations) => {
      const live = locations.filter((location) => location.kind === "volume");
      const present = new Set(live.map((location) => location.id));
      // Retain bounded disconnected entries so a disappearing device does not
      // look like an empty list or silently become a different device.
      const missing = this.snapshot.locations.filter((location) => !present.has(location.id))
        .map((location) => ({ ...location, available: false }));
      this.publish({ ...this.snapshot, locations: [...live, ...missing].slice(0, 32), loading: false, failed: false });
    }).catch(() => {
      this.publish({ ...this.snapshot, locations: this.snapshot.locations.map((location) => ({ ...location, available: false })), loading: false, failed: true });
    }).finally(() => { this.inFlight = null; });
    return this.inFlight;
  };
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

const mountedLocations = new MountedLocationProjection();
const inactiveSubscription = () => () => undefined;
export function useMountedLocations(enabled: boolean) {
  const snapshot = useSyncExternalStore(enabled ? mountedLocations.subscribe : inactiveSubscription, mountedLocations.getSnapshot, mountedLocations.getSnapshot);
  return { ...snapshot, refresh: mountedLocations.refresh, eject: mountedLocations.eject };
}
